//! One-shot orchestration. Watch and scheduled modes are intentionally deferred.

use serde::Serialize;
use sorter_core::{MatchEngine, PlannedMove, Rule};
use sorter_fs::{execute_move, CollisionPolicy, MoveOutcome};
use sorter_storage::{HistoryStore, OperationState};
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
    let operation_id = history
        .map(|store| store.create_operation(&plan.source, &plan.destination, &plan.rule_id))
        .transpose()?;
    if let (Some(store), Some(id)) = (history, operation_id) {
        store.update_operation(id, OperationState::InProgress, None)?;
    }

    match execute_move(plan, collision_policy) {
        Ok(MoveOutcome::Moved { destination }) => {
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
            let state = if matches!(
                move_error,
                sorter_fs::FsError::SourceRemovalFailed { .. }
                    | sorter_fs::FsError::PublishedSyncFailed { .. }
            ) {
                OperationState::RecoveryRequired
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
}
