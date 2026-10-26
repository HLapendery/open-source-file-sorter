//! One-shot orchestration. Watch and scheduled modes are intentionally deferred.

use serde::Serialize;
use sorter_core::{MatchEngine, PlannedMove, Rule};
use sorter_fs::{
    execute_move_with_hooks, files_equal, publish_verified_temporary, remove_verified_source,
    same_file_identity, CollisionPolicy, FsError, MoveOutcome, TransferEvent, TransferHooks,
    TransferKind,
};
use sorter_storage::{HistoryStore, OperationRecord, OperationState};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{error, info};
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("source directory does not exist: {0}")]
    SourceMissing(PathBuf),
    #[error("source is not a directory: {0}")]
    SourceNotDirectory(PathBuf),
    #[error("source directory must not be a symbolic link: {0}")]
    SourceIsSymlink(PathBuf),
    #[error("destination for rule {rule_id} is inside recursive source: {destination}")]
    RecursiveDestinationLoop {
        rule_id: String,
        destination: PathBuf,
    },
    #[error("failed to inspect source {path}: {source}")]
    InspectSource {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    Core(#[from] sorter_core::CoreError),
    #[error(transparent)]
    Storage(#[from] sorter_storage::StorageError),
    #[error(transparent)]
    Filesystem(#[from] FsError),
}

#[derive(Debug, Serialize)]
pub struct RunReport {
    pub source: PathBuf,
    pub dry_run: bool,
    pub scanned: u64,
    pub matched: u64,
    pub moved: u64,
    pub skipped: u64,
    pub failed: u64,
    pub items: Vec<ItemReport>,
    pub scan_errors: Vec<String>,
}

impl RunReport {
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.failed > 0 || !self.scan_errors.is_empty()
    }
}

#[derive(Debug, Serialize)]
pub struct ItemReport {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub rule_id: String,
    pub status: ItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Planned,
    Moved,
    SkippedCollision,
    Failed,
}

pub struct OneShotOptions<'a> {
    pub source: &'a Path,
    pub recursive: bool,
    pub dry_run: bool,
    pub collision_policy: CollisionPolicy,
    pub rules: &'a [Rule],
}

pub fn run_once(
    options: &OneShotOptions<'_>,
    history: Option<&HistoryStore>,
) -> Result<RunReport, RuntimeError> {
    validate_source(options.source)?;
    validate_no_recursive_loops(options)?;
    let engine = MatchEngine::compile(options.rules)?;
    let max_depth = if options.recursive { usize::MAX } else { 1 };
    let mut report = RunReport {
        source: options.source.to_path_buf(),
        dry_run: options.dry_run,
        scanned: 0,
        matched: 0,
        moved: 0,
        skipped: 0,
        failed: 0,
        items: Vec::new(),
        scan_errors: Vec::new(),
    };

    for entry in WalkDir::new(options.source)
        .follow_links(false)
        .min_depth(1)
        .max_depth(max_depth)
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.scan_errors.push(error.to_string());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        report.scanned += 1;
        let Some(plan) = engine.plan(entry.path())? else {
            continue;
        };
        report.matched += 1;
        if options.dry_run {
            report
                .items
                .push(item_from_plan(&plan, ItemStatus::Planned, None));
            continue;
        }
        execute_and_record(&plan, options.collision_policy, history, &mut report)?;
    }

    Ok(report)
}

fn execute_and_record(
    plan: &PlannedMove,
    collision_policy: CollisionPolicy,
    history: Option<&HistoryStore>,
    report: &mut RunReport,
) -> Result<(), RuntimeError> {
    let journal_source = fs::canonicalize(&plan.source).unwrap_or_else(|_| plan.source.clone());
    let operation_id = if let Some(store) = history {
        match store.create_operation(&journal_source, &plan.destination, &plan.rule_id) {
            Ok(id) => Some(id),
            Err(error) => {
                let message = error.to_string();
                error!(source = %plan.source.display(), error = %message, "could not journal file move");
                report.failed += 1;
                report
                    .items
                    .push(item_from_plan(plan, ItemStatus::Failed, Some(message)));
                return Ok(());
            }
        }
    } else {
        None
    };
    let mut hooks = JournalHooks {
        history,
        operation_id,
    };
    let temporary_tag = operation_id.map_or_else(
        || format!("{}-untracked", std::process::id()),
        |id| id.to_string(),
    );

    match execute_move_with_hooks(plan, collision_policy, &temporary_tag, false, &mut hooks) {
        Ok(MoveOutcome::Moved { destination, .. }) => {
            if let (Some(store), Some(id)) = (history, operation_id) {
                store.update_destination(id, &destination)?;
                store.update_operation(id, OperationState::Completed, None)?;
            }
            info!(source = %plan.source.display(), destination = %destination.display(), rule = %plan.rule_id, "file moved");
            report.moved += 1;
            report.items.push(ItemReport {
                source: plan.source.clone(),
                destination,
                rule_id: plan.rule_id.clone(),
                status: ItemStatus::Moved,
                error: None,
            });
        }
        Ok(MoveOutcome::SkippedCollision { destination }) => {
            if let (Some(store), Some(id)) = (history, operation_id) {
                store.update_operation(id, OperationState::Skipped, None)?;
            }
            report.skipped += 1;
            report.items.push(ItemReport {
                source: plan.source.clone(),
                destination,
                rule_id: plan.rule_id.clone(),
                status: ItemStatus::SkippedCollision,
                error: None,
            });
        }
        Err(move_error) => {
            let message = move_error.to_string();
            let state = if move_error.needs_recovery() {
                if let (Some(store), Some(id)) = (history, operation_id) {
                    if store.operation(id)?.state == OperationState::Published {
                        OperationState::Published
                    } else {
                        OperationState::RecoveryRequired
                    }
                } else {
                    OperationState::RecoveryRequired
                }
            } else {
                OperationState::Failed
            };
            if let (Some(store), Some(id)) = (history, operation_id) {
                store.update_operation(id, state, Some(&message))?;
            }
            error!(source = %plan.source.display(), error = %message, "file move failed");
            report.failed += 1;
            report
                .items
                .push(item_from_plan(plan, ItemStatus::Failed, Some(message)));
        }
    }
    Ok(())
}

struct JournalHooks<'a> {
    history: Option<&'a HistoryStore>,
    operation_id: Option<i64>,
}

impl TransferHooks for JournalHooks<'_> {
    fn record(&mut self, event: TransferEvent) -> Result<(), String> {
        let (Some(history), Some(id)) = (self.history, self.operation_id) else {
            return Ok(());
        };
        let result = match event {
            TransferEvent::Prepared {
                destination,
                temporary_path,
                kind,
                source,
            } => {
                history.record_prepared(id, &destination, temporary_path.as_deref(), kind, &source)
            }
            TransferEvent::Copied => history.update_operation(id, OperationState::Copied, None),
            TransferEvent::Verified => history.update_operation(id, OperationState::Verified, None),
            TransferEvent::Published => {
                history.update_operation(id, OperationState::Published, None)
            }
            TransferEvent::SourceRemoved => {
                history.update_operation(id, OperationState::Completed, None)
            }
        };
        result.map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RecoveryOptions {
    pub apply: bool,
    pub operation_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RecoveryReport {
    pub apply: bool,
    pub inspected: u64,
    pub completed: u64,
    pub closed_without_changes: u64,
    pub needs_review: u64,
    pub items: Vec<RecoveryItem>,
}

#[derive(Debug, Serialize)]
pub struct RecoveryItem {
    pub operation_id: i64,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub previous_state: OperationState,
    pub status: RecoveryStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStatus {
    WouldComplete,
    Completed,
    WouldCloseWithoutChanges,
    ClosedWithoutChanges,
    NeedsReview,
}

pub fn recover_operations(
    history: &HistoryStore,
    options: RecoveryOptions,
) -> Result<RecoveryReport, RuntimeError> {
    let operations = history.recoverable(options.operation_id)?;
    let mut report = RecoveryReport {
        apply: options.apply,
        inspected: 0,
        completed: 0,
        closed_without_changes: 0,
        needs_review: 0,
        items: Vec::with_capacity(operations.len()),
    };
    for operation in operations {
        report.inspected += 1;
        let item = recover_one(history, &operation, options.apply)?;
        match item.status {
            RecoveryStatus::Completed => report.completed += 1,
            RecoveryStatus::ClosedWithoutChanges => report.closed_without_changes += 1,
            RecoveryStatus::NeedsReview => report.needs_review += 1,
            RecoveryStatus::WouldComplete | RecoveryStatus::WouldCloseWithoutChanges => {}
        }
        report.items.push(item);
    }
    Ok(report)
}

fn recover_one(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
) -> Result<RecoveryItem, RuntimeError> {
    let source_exists = match regular_file_exists(&operation.source) {
        Ok(value) => value,
        Err(message) => return needs_review(history, operation, apply, message),
    };
    let destination_exists = match regular_file_exists(&operation.destination) {
        Ok(value) => value,
        Err(message) => return needs_review(history, operation, apply, message),
    };
    let temporary_exists = match operation.temporary_path.as_deref() {
        Some(path) => match regular_file_exists(path) {
            Ok(value) => value,
            Err(message) => return needs_review(history, operation, apply, message),
        },
        None => false,
    };

    match (source_exists, destination_exists, temporary_exists) {
        (true, true, _) => recover_source_and_destination(history, operation, apply),
        (true, false, true) => recover_verified_temporary(history, operation, apply),
        (true, false, false) => close_unchanged_operation(history, operation, apply),
        (false, true, _) if operation.state == OperationState::Published => {
            complete_published_destination(history, operation, apply, temporary_exists)
        }
        (false, true, _) => needs_review(
            history,
            operation,
            apply,
            "the source is missing but publication was not durably journaled".into(),
        ),
        (false, false, true) => needs_review(
            history,
            operation,
            apply,
            "only the private temporary file remains; source disappearance is ambiguous".into(),
        ),
        (false, false, false) => needs_review(
            history,
            operation,
            apply,
            "source, destination, and temporary file are all missing".into(),
        ),
    }
}

fn recover_source_and_destination(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
) -> Result<RecoveryItem, RuntimeError> {
    let content_matches = match files_equal(&operation.source, &operation.destination) {
        Ok(value) => value,
        Err(error) => return needs_review(history, operation, apply, error.to_string()),
    };
    if !content_matches {
        return needs_review(
            history,
            operation,
            apply,
            "source and destination differ; neither file was changed".into(),
        );
    }

    let publication_is_ours = if operation.state == OperationState::Published {
        true
    } else if operation.transfer_kind == Some(TransferKind::HardLink) {
        same_file_identity(&operation.source, &operation.destination).unwrap_or(false)
    } else if let Some(temporary) = operation.temporary_path.as_deref() {
        regular_file_exists(temporary).unwrap_or(false)
            && same_file_identity(temporary, &operation.destination).unwrap_or(false)
    } else {
        false
    };
    if !publication_is_ours {
        return needs_review(
            history,
            operation,
            apply,
            "identical files exist, but journal and file identity do not prove publication".into(),
        );
    }

    if !apply {
        return Ok(recovery_item(
            operation,
            RecoveryStatus::WouldComplete,
            "verified publication can be completed safely".into(),
        ));
    }
    cleanup_temporary_if_published(operation)?;
    history.update_operation(operation.id, OperationState::Published, None)?;
    remove_verified_source(&operation.source, &operation.destination)?;
    history.update_operation(operation.id, OperationState::Completed, None)?;
    Ok(recovery_item(
        operation,
        RecoveryStatus::Completed,
        "verified destination retained and source removed".into(),
    ))
}

fn recover_verified_temporary(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
) -> Result<RecoveryItem, RuntimeError> {
    let temporary = operation
        .temporary_path
        .as_deref()
        .expect("temporary existence was checked");
    match files_equal(&operation.source, temporary) {
        Ok(true) => {}
        Ok(false) => {
            return needs_review(
                history,
                operation,
                apply,
                "temporary file is incomplete or differs from source".into(),
            );
        }
        Err(error) => return needs_review(history, operation, apply, error.to_string()),
    }
    if !apply {
        return Ok(recovery_item(
            operation,
            RecoveryStatus::WouldComplete,
            "verified temporary file can be published safely".into(),
        ));
    }
    if let Err(error) = publish_verified_temporary(temporary, &operation.destination) {
        return needs_review(history, operation, true, error.to_string());
    }
    history.update_operation(operation.id, OperationState::Published, None)?;
    if let Err(error) = fs::remove_file(temporary) {
        return needs_review(
            history,
            operation,
            true,
            format!("destination is published; temporary cleanup failed: {error}"),
        );
    }
    if let Err(error) = remove_verified_source(&operation.source, &operation.destination) {
        return needs_review(history, operation, true, error.to_string());
    }
    history.update_operation(operation.id, OperationState::Completed, None)?;
    Ok(recovery_item(
        operation,
        RecoveryStatus::Completed,
        "verified temporary file published and source removed".into(),
    ))
}

fn close_unchanged_operation(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
) -> Result<RecoveryItem, RuntimeError> {
    let message = "no destination data exists; the unchanged source was retained";
    if apply {
        history.update_operation(operation.id, OperationState::Failed, Some(message))?;
    }
    Ok(recovery_item(
        operation,
        if apply {
            RecoveryStatus::ClosedWithoutChanges
        } else {
            RecoveryStatus::WouldCloseWithoutChanges
        },
        message.into(),
    ))
}

fn complete_published_destination(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
    temporary_exists: bool,
) -> Result<RecoveryItem, RuntimeError> {
    if let Some(expected) = &operation.source_fingerprint {
        match fs::metadata(&operation.destination) {
            Ok(metadata) if metadata.len() == expected.size => {}
            Ok(_) => {
                return needs_review(
                    history,
                    operation,
                    apply,
                    "published destination size differs from journal metadata".into(),
                );
            }
            Err(error) => return needs_review(history, operation, apply, error.to_string()),
        }
    }
    if !apply {
        return Ok(recovery_item(
            operation,
            RecoveryStatus::WouldComplete,
            "source removal and destination publication are already reflected on disk".into(),
        ));
    }
    if temporary_exists {
        cleanup_temporary_if_published(operation)?;
    }
    history.update_operation(operation.id, OperationState::Completed, None)?;
    Ok(recovery_item(
        operation,
        RecoveryStatus::Completed,
        "journal marked complete; published destination was preserved".into(),
    ))
}

fn cleanup_temporary_if_published(operation: &OperationRecord) -> Result<(), RuntimeError> {
    let Some(temporary) = operation.temporary_path.as_deref() else {
        return Ok(());
    };
    if !temporary.exists() {
        return Ok(());
    }
    let matches = same_file_identity(temporary, &operation.destination)
        .or_else(|_| files_equal(temporary, &operation.destination))
        .unwrap_or(false);
    if matches {
        fs::remove_file(temporary).map_err(|error| RuntimeError::InspectSource {
            path: temporary.to_path_buf(),
            source: error,
        })?;
    }
    Ok(())
}

fn needs_review(
    history: &HistoryStore,
    operation: &OperationRecord,
    apply: bool,
    message: String,
) -> Result<RecoveryItem, RuntimeError> {
    if apply && operation.state != OperationState::Published {
        history.update_operation(
            operation.id,
            OperationState::RecoveryRequired,
            Some(&message),
        )?;
    }
    Ok(recovery_item(
        operation,
        RecoveryStatus::NeedsReview,
        message,
    ))
}

fn recovery_item(
    operation: &OperationRecord,
    status: RecoveryStatus,
    message: String,
) -> RecoveryItem {
    RecoveryItem {
        operation_id: operation.id,
        source: operation.source.clone(),
        destination: operation.destination.clone(),
        previous_state: operation.state,
        status,
        message,
    }
}

fn regular_file_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "refusing recovery through symbolic link {}",
            path.display()
        )),
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(format!(
            "recovery path is not a regular file: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not inspect {}: {error}", path.display())),
    }
}

fn item_from_plan(plan: &PlannedMove, status: ItemStatus, error: Option<String>) -> ItemReport {
    ItemReport {
        source: plan.source.clone(),
        destination: plan.destination.clone(),
        rule_id: plan.rule_id.clone(),
        status,
        error,
    }
}

fn validate_source(source: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            RuntimeError::SourceMissing(source.to_path_buf())
        } else {
            RuntimeError::InspectSource {
                path: source.to_path_buf(),
                source: error,
            }
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::SourceIsSymlink(source.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(RuntimeError::SourceNotDirectory(source.to_path_buf()));
    }
    Ok(())
}

fn validate_no_recursive_loops(options: &OneShotOptions<'_>) -> Result<(), RuntimeError> {
    if !options.recursive {
        return Ok(());
    }
    for rule in options.rules {
        if rule.destination.starts_with(options.source) {
            return Err(RuntimeError::RecursiveDestinationLoop {
                rule_id: rule.id.clone(),
                destination: rule.destination.clone(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorter_core::MatchSpec;
    use sorter_fs::FileFingerprint;
    use std::fs;
    use tempfile::tempdir;

    fn rule(destination: PathBuf) -> Rule {
        Rule {
            id: "text".into(),
            enabled: true,
            priority: 1,
            destination,
            match_spec: MatchSpec {
                extensions: vec!["txt".into()],
                ..MatchSpec::default()
            },
        }
    }

    #[test]
    fn dry_run_does_not_change_files() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("notes.txt"), b"hello").unwrap();
        let rules = [rule(destination.clone())];
        let report = run_once(
            &OneShotOptions {
                source: &source,
                recursive: false,
                dry_run: true,
                collision_policy: CollisionPolicy::Error,
                rules: &rules,
            },
            None,
        )
        .unwrap();
        assert_eq!(report.matched, 1);
        assert!(source.join("notes.txt").exists());
        assert!(!destination.exists());
    }

    #[test]
    fn recursive_scan_moves_nested_files() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/notes.txt"), b"hello").unwrap();
        let rules = [rule(destination.clone())];
        let report = run_once(
            &OneShotOptions {
                source: &source,
                recursive: true,
                dry_run: false,
                collision_policy: CollisionPolicy::Error,
                rules: &rules,
            },
            None,
        )
        .unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(fs::read(destination.join("notes.txt")).unwrap(), b"hello");
    }

    #[test]
    fn one_failure_does_not_prevent_other_files() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(source.join("a.txt"), b"new-a").unwrap();
        fs::write(source.join("b.txt"), b"new-b").unwrap();
        fs::write(destination.join("a.txt"), b"old-a").unwrap();
        let rules = [rule(destination.clone())];
        let report = run_once(
            &OneShotOptions {
                source: &source,
                recursive: false,
                dry_run: false,
                collision_policy: CollisionPolicy::Error,
                rules: &rules,
            },
            None,
        )
        .unwrap();
        assert_eq!(report.failed, 1);
        assert_eq!(report.moved, 1);
        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"old-a");
    }

    fn fingerprint(size: u64) -> FileFingerprint {
        FileFingerprint {
            size,
            modified_unix_ns: None,
            device: None,
            file_id: None,
        }
    }

    #[test]
    fn recovery_completes_journaled_publication_idempotently() {
        let root = tempdir().unwrap();
        let source = root.path().join("source.txt");
        let destination = root.path().join("destination.txt");
        fs::write(&source, b"recover me").unwrap();
        fs::hard_link(&source, &destination).unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let id = store
            .create_operation(&source, &destination, "text")
            .unwrap();
        store
            .record_prepared(
                id,
                &destination,
                None,
                TransferKind::HardLink,
                &fingerprint(10),
            )
            .unwrap();
        store
            .update_operation(id, OperationState::Published, None)
            .unwrap();

        let preview = recover_operations(
            &store,
            RecoveryOptions {
                apply: false,
                operation_id: None,
            },
        )
        .unwrap();
        assert_eq!(preview.items[0].status, RecoveryStatus::WouldComplete);
        assert!(source.exists());

        let applied = recover_operations(
            &store,
            RecoveryOptions {
                apply: true,
                operation_id: None,
            },
        )
        .unwrap();
        assert_eq!(applied.completed, 1);
        assert!(!source.exists());
        assert_eq!(fs::read(&destination).unwrap(), b"recover me");
        assert_eq!(
            store.operation(id).unwrap().state,
            OperationState::Completed
        );

        let repeated = recover_operations(
            &store,
            RecoveryOptions {
                apply: true,
                operation_id: None,
            },
        )
        .unwrap();
        assert_eq!(repeated.inspected, 0);
    }

    #[test]
    fn recovery_publishes_only_a_verified_matching_temporary_file() {
        let root = tempdir().unwrap();
        let source = root.path().join("source.txt");
        let destination = root.path().join("destination.txt");
        let temporary = root.path().join(".destination.partial");
        fs::write(&source, b"verified").unwrap();
        fs::write(&temporary, b"verified").unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let id = store
            .create_operation(&source, &destination, "text")
            .unwrap();
        store
            .record_prepared(
                id,
                &destination,
                Some(&temporary),
                TransferKind::StagedCopy,
                &fingerprint(8),
            )
            .unwrap();
        store
            .update_operation(id, OperationState::Verified, None)
            .unwrap();

        let report = recover_operations(
            &store,
            RecoveryOptions {
                apply: true,
                operation_id: Some(id),
            },
        )
        .unwrap();
        assert_eq!(report.completed, 1);
        assert!(!source.exists());
        assert!(!temporary.exists());
        assert_eq!(fs::read(destination).unwrap(), b"verified");
    }

    #[test]
    fn recovery_preserves_ambiguous_files() {
        let root = tempdir().unwrap();
        let source = root.path().join("source.txt");
        let destination = root.path().join("destination.txt");
        fs::write(&source, b"source").unwrap();
        fs::write(&destination, b"different").unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let id = store
            .create_operation(&source, &destination, "text")
            .unwrap();
        store
            .record_prepared(
                id,
                &destination,
                None,
                TransferKind::HardLink,
                &fingerprint(6),
            )
            .unwrap();
        store
            .update_operation(id, OperationState::RecoveryRequired, None)
            .unwrap();

        let report = recover_operations(
            &store,
            RecoveryOptions {
                apply: true,
                operation_id: Some(id),
            },
        )
        .unwrap();
        assert_eq!(report.needs_review, 1);
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(destination).unwrap(), b"different");
    }
}
