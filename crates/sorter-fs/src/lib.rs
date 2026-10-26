//! Conservative file transfers with explicit, journalable state transitions.

use filetime::{set_file_times, FileTime};
use serde::{Deserialize, Serialize};
use sorter_core::PlannedMove;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tracing::warn;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const COPY_BUFFER_SIZE: usize = 128 * 1024;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CollisionPolicy {
    #[default]
    Skip,
    RenameWithSuffix,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferKind {
    HardLink,
    StagedCopy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileFingerprint {
    pub size: u64,
    pub modified_unix_ns: Option<i128>,
    pub device: Option<u64>,
    pub file_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferEvent {
    Prepared {
        destination: PathBuf,
        temporary_path: Option<PathBuf>,
        kind: TransferKind,
        source: FileFingerprint,
    },
    Copied,
    Verified,
    Published,
    SourceRemoved,
}

impl TransferEvent {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Prepared { .. } => "prepared",
            Self::Copied => "copied",
            Self::Verified => "verified",
            Self::Published => "published",
            Self::SourceRemoved => "source_removed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    BeforeCopy,
    DuringCopy,
    AfterCopy,
    AfterVerification,
    BeforePublication,
    AfterPublication,
    BeforeSourceDeletion,
    AfterSourceDeletion,
}

impl FaultPoint {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BeforeCopy => "before_copy",
            Self::DuringCopy => "during_copy",
            Self::AfterCopy => "after_copy",
            Self::AfterVerification => "after_verification",
            Self::BeforePublication => "before_publication",
            Self::AfterPublication => "after_publication",
            Self::BeforeSourceDeletion => "before_source_deletion",
            Self::AfterSourceDeletion => "after_source_deletion",
        }
    }
}

pub trait TransferHooks {
    fn record(&mut self, event: TransferEvent) -> Result<(), String>;

    fn checkpoint(&mut self, _point: FaultPoint) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
pub struct NoopTransferHooks;

impl TransferHooks for NoopTransferHooks {
    fn record(&mut self, _event: TransferEvent) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoveOutcome {
    Moved {
        destination: PathBuf,
        kind: TransferKind,
    },
    SkippedCollision {
        destination: PathBuf,
    },
}

#[derive(Debug, Error)]
pub enum FsError {
    #[error("source does not exist: {0}")]
    SourceMissing(PathBuf),
    #[error("source is not a regular file: {0}")]
    SourceNotRegular(PathBuf),
    #[error("symbolic links are not followed: {0}")]
    SymbolicLink(PathBuf),
    #[error("destination directories must not be symbolic links: {0}")]
    SymbolicLinkDestination(PathBuf),
    #[error("destination already exists: {0}")]
    Collision(PathBuf),
    #[error("temporary transfer path already exists: {0}")]
    TemporaryCollision(PathBuf),
    #[error("destination has no parent: {0}")]
    InvalidDestination(PathBuf),
    #[error("source and destination refer to the same path: {0}")]
    SamePath(PathBuf),
    #[error("source changed while it was being transferred: {0}")]
    SourceChanged(PathBuf),
    #[error("copy verification failed: {source_path} != {temporary_path}")]
    VerificationFailed {
        source_path: PathBuf,
        temporary_path: PathBuf,
    },
    #[error("filesystem operation {operation} failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("staged transfer failed during {operation} for {path}: {source}")]
    StagedTransfer {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("destination was published but the source could not be removed; both files were preserved: {source_path} -> {destination_path}: {source}")]
    SourceRemovalFailed {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("destination was published but could not be synchronized; both files were preserved: {source_path} -> {destination_path}: {source}")]
    PublishedSyncFailed {
        source_path: PathBuf,
        destination_path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("transfer hook failed at {point}: {message}")]
    HookFailed {
        point: &'static str,
        message: String,
    },
}

impl FsError {
    #[must_use]
    pub const fn needs_recovery(&self) -> bool {
        matches!(
            self,
            Self::TemporaryCollision(_)
                | Self::SourceChanged(_)
                | Self::VerificationFailed { .. }
                | Self::StagedTransfer { .. }
                | Self::SourceRemovalFailed { .. }
                | Self::PublishedSyncFailed { .. }
                | Self::HookFailed { .. }
        )
    }
}

pub fn execute_move(
    planned: &PlannedMove,
    collision_policy: CollisionPolicy,
) -> Result<MoveOutcome, FsError> {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_tag = format!("{}-{sequence}", std::process::id());
    execute_move_with_hooks(
        planned,
        collision_policy,
        &temporary_tag,
        false,
        &mut NoopTransferHooks,
    )
}

pub fn execute_move_with_hooks(
    planned: &PlannedMove,
    collision_policy: CollisionPolicy,
    temporary_tag: &str,
    force_staged_copy: bool,
    hooks: &mut impl TransferHooks,
) -> Result<MoveOutcome, FsError> {
    let source_metadata = validate_source(&planned.source)?;
    if planned.source == planned.destination {
        return Err(FsError::SamePath(planned.source.clone()));
    }
    let source_fingerprint = fingerprint(&source_metadata);
    let mut destination = resolve_destination(&planned.destination, collision_policy)?;
    if destination.exists() {
        return Ok(MoveOutcome::SkippedCollision { destination });
    }

    loop {
        let parent = prepare_destination_parent(&destination)?.to_path_buf();
        if !force_staged_copy {
            record(
                hooks,
                TransferEvent::Prepared {
                    destination: destination.clone(),
                    temporary_path: None,
                    kind: TransferKind::HardLink,
                    source: source_fingerprint.clone(),
                },
            )?;
            match fs::hard_link(&planned.source, &destination) {
                Ok(()) => {
                    return finish_published_hard_link(
                        planned,
                        destination,
                        &parent,
                        &source_fingerprint,
                        hooks,
                    );
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    match collision_policy {
                        CollisionPolicy::Skip => {
                            return Ok(MoveOutcome::SkippedCollision { destination });
                        }
                        CollisionPolicy::Error => return Err(FsError::Collision(destination)),
                        CollisionPolicy::RenameWithSuffix => {
                            destination =
                                resolve_destination(&planned.destination, collision_policy)?;
                            continue;
                        }
                    }
                }
                Err(error) if is_cross_device(&error) => {}
                Err(source) => return Err(io_error("publish destination", &destination, source)),
            }
        }
        return staged_copy(
            planned,
            destination,
            &parent,
            collision_policy,
            temporary_tag,
            &source_fingerprint,
            hooks,
        );
    }
}

fn finish_published_hard_link(
    planned: &PlannedMove,
    destination: PathBuf,
    parent: &Path,
    source_fingerprint: &FileFingerprint,
    hooks: &mut impl TransferHooks,
) -> Result<MoveOutcome, FsError> {
    File::open(&destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::PublishedSyncFailed {
            source_path: planned.source.clone(),
            destination_path: destination.clone(),
            source,
        })?;
    sync_directory(parent);
    record(hooks, TransferEvent::Published)?;
    checkpoint(hooks, FaultPoint::AfterPublication)?;
    checkpoint(hooks, FaultPoint::BeforeSourceDeletion)?;
    ensure_source_unchanged(&planned.source, source_fingerprint)?;
    fs::remove_file(&planned.source).map_err(|source| FsError::SourceRemovalFailed {
        source_path: planned.source.clone(),
        destination_path: destination.clone(),
        source,
    })?;
    sync_source_parent(&planned.source);
    checkpoint(hooks, FaultPoint::AfterSourceDeletion)?;
    record(hooks, TransferEvent::SourceRemoved)?;
    Ok(MoveOutcome::Moved {
        destination,
        kind: TransferKind::HardLink,
    })
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn staged_copy(
    planned: &PlannedMove,
    mut destination: PathBuf,
    parent: &Path,
    collision_policy: CollisionPolicy,
    temporary_tag: &str,
    source_fingerprint: &FileFingerprint,
    hooks: &mut impl TransferHooks,
) -> Result<MoveOutcome, FsError> {
    let temporary_path = temporary_path(&destination, temporary_tag);
    record(
        hooks,
        TransferEvent::Prepared {
            destination: destination.clone(),
            temporary_path: Some(temporary_path.clone()),
            kind: TransferKind::StagedCopy,
            source: source_fingerprint.clone(),
        },
    )?;
    checkpoint(hooks, FaultPoint::BeforeCopy)?;

    let mut source_file = File::open(&planned.source)
        .map_err(|source| staged_error("open source", &planned.source, source))?;
    ensure_file_unchanged(&source_file, &planned.source, source_fingerprint)?;
    let mut temporary_file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&temporary_path)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::AlreadyExists {
                FsError::TemporaryCollision(temporary_path.clone())
            } else {
                staged_error("create temporary file", &temporary_path, source)
            }
        })?;

    copy_stream(
        &mut source_file,
        &mut temporary_file,
        &temporary_path,
        hooks,
    )?;
    let source_metadata = source_file
        .metadata()
        .map_err(|source| staged_error("read source metadata", &planned.source, source))?;
    set_prepublication_permissions(&temporary_file, &source_metadata, &temporary_path)?;
    let accessed = FileTime::from_last_access_time(&source_metadata);
    let modified = FileTime::from_last_modification_time(&source_metadata);
    set_file_times(&temporary_path, accessed, modified)
        .map_err(|source| staged_error("set temporary timestamps", &temporary_path, source))?;
    temporary_file
        .sync_all()
        .map_err(|source| staged_error("synchronize temporary file", &temporary_path, source))?;
    ensure_file_unchanged(&source_file, &planned.source, source_fingerprint)?;
    record(hooks, TransferEvent::Copied)?;
    checkpoint(hooks, FaultPoint::AfterCopy)?;

    verify_open_files(
        &mut source_file,
        &mut temporary_file,
        &planned.source,
        &temporary_path,
    )?;
    ensure_file_unchanged(&source_file, &planned.source, source_fingerprint)?;
    record(hooks, TransferEvent::Verified)?;
    checkpoint(hooks, FaultPoint::AfterVerification)?;
    checkpoint(hooks, FaultPoint::BeforePublication)?;

    loop {
        match fs::hard_link(&temporary_path, &destination) {
            Ok(()) => break,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                match collision_policy {
                    CollisionPolicy::Skip | CollisionPolicy::Error => {
                        // The private temporary file is safe to discard because
                        // the source still exists and has been revalidated.
                        fs::remove_file(&temporary_path).map_err(|source| {
                            staged_error(
                                "remove temporary file after collision",
                                &temporary_path,
                                source,
                            )
                        })?;
                        return if collision_policy == CollisionPolicy::Skip {
                            Ok(MoveOutcome::SkippedCollision { destination })
                        } else {
                            Err(FsError::Collision(destination))
                        };
                    }
                    CollisionPolicy::RenameWithSuffix => {
                        destination = resolve_destination(&planned.destination, collision_policy)?;
                        record(
                            hooks,
                            TransferEvent::Prepared {
                                destination: destination.clone(),
                                temporary_path: Some(temporary_path.clone()),
                                kind: TransferKind::StagedCopy,
                                source: source_fingerprint.clone(),
                            },
                        )?;
                        record(hooks, TransferEvent::Copied)?;
                        record(hooks, TransferEvent::Verified)?;
                    }
                }
            }
            Err(source) => {
                return Err(staged_error(
                    "publish verified temporary file",
                    &destination,
                    source,
                ));
            }
        }
    }
    File::open(&destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::PublishedSyncFailed {
            source_path: planned.source.clone(),
            destination_path: destination.clone(),
            source,
        })?;
    sync_directory(parent);
    record(hooks, TransferEvent::Published)?;
    checkpoint(hooks, FaultPoint::AfterPublication)?;

    // Publication uses a hard link so recovery can observe both names. Once
    // journaled as published, the private name is no longer needed.
    fs::remove_file(&temporary_path).map_err(|source| {
        staged_error("remove published temporary file", &temporary_path, source)
    })?;
    set_postpublication_permissions(&destination, &source_metadata)?;
    checkpoint(hooks, FaultPoint::BeforeSourceDeletion)?;
    ensure_source_unchanged(&planned.source, source_fingerprint)?;
    fs::remove_file(&planned.source).map_err(|source| FsError::SourceRemovalFailed {
        source_path: planned.source.clone(),
        destination_path: destination.clone(),
        source,
    })?;
    sync_source_parent(&planned.source);
    checkpoint(hooks, FaultPoint::AfterSourceDeletion)?;
    record(hooks, TransferEvent::SourceRemoved)?;

    Ok(MoveOutcome::Moved {
        destination,
        kind: TransferKind::StagedCopy,
    })
}

fn copy_stream(
    source: &mut File,
    destination: &mut File,
    destination_path: &Path,
    hooks: &mut impl TransferHooks,
) -> Result<(), FsError> {
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    let mut checkpoint_called = false;
    loop {
        let count = source
            .read(&mut buffer)
            .map_err(|error| staged_error("read source during copy", destination_path, error))?;
        if count == 0 {
            break;
        }
        destination
            .write_all(&buffer[..count])
            .map_err(|error| staged_error("write temporary file", destination_path, error))?;
        if !checkpoint_called {
            checkpoint(hooks, FaultPoint::DuringCopy)?;
            checkpoint_called = true;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn set_prepublication_permissions(
    temporary: &File,
    source_metadata: &fs::Metadata,
    temporary_path: &Path,
) -> Result<(), FsError> {
    temporary
        .set_permissions(source_metadata.permissions())
        .map_err(|source| staged_error("set temporary permissions", temporary_path, source))
}

#[cfg(windows)]
fn set_prepublication_permissions(
    _temporary: &File,
    _source_metadata: &fs::Metadata,
    _temporary_path: &Path,
) -> Result<(), FsError> {
    // A read-only Windows file cannot be unlinked. Apply its permissions only
    // after the private hard-link name has been removed.
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
fn set_postpublication_permissions(
    _destination: &Path,
    _source_metadata: &fs::Metadata,
) -> Result<(), FsError> {
    Ok(())
}

#[cfg(windows)]
fn set_postpublication_permissions(
    destination: &Path,
    source_metadata: &fs::Metadata,
) -> Result<(), FsError> {
    fs::set_permissions(destination, source_metadata.permissions())
        .map_err(|source| staged_error("set destination permissions", destination, source))?;
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| staged_error("synchronize destination metadata", destination, source))
}

fn verify_open_files(
    source: &mut File,
    temporary: &mut File,
    source_path: &Path,
    temporary_path: &Path,
) -> Result<(), FsError> {
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| staged_error("rewind source", source_path, error))?;
    temporary
        .seek(SeekFrom::Start(0))
        .map_err(|error| staged_error("rewind temporary file", temporary_path, error))?;
    let mut source_buffer = vec![0_u8; COPY_BUFFER_SIZE];
    let mut temporary_buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let source_count = source
            .read(&mut source_buffer)
            .map_err(|error| staged_error("verify source", source_path, error))?;
        let temporary_count = temporary
            .read(&mut temporary_buffer)
            .map_err(|error| staged_error("verify temporary file", temporary_path, error))?;
        if source_count != temporary_count
            || source_buffer[..source_count] != temporary_buffer[..temporary_count]
        {
            return Err(FsError::VerificationFailed {
                source_path: source_path.to_path_buf(),
                temporary_path: temporary_path.to_path_buf(),
            });
        }
        if source_count == 0 {
            return Ok(());
        }
    }
}

pub fn files_equal(left: &Path, right: &Path) -> Result<bool, FsError> {
    let left_metadata =
        fs::metadata(left).map_err(|source| io_error("read file metadata", left, source))?;
    let right_metadata =
        fs::metadata(right).map_err(|source| io_error("read file metadata", right, source))?;
    if left_metadata.len() != right_metadata.len() {
        return Ok(false);
    }
    let mut left = File::open(left).map_err(|source| io_error("open file", left, source))?;
    let mut right = File::open(right).map_err(|source| io_error("open file", right, source))?;
    let mut left_buffer = vec![0_u8; COPY_BUFFER_SIZE];
    let mut right_buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let left_count = left
            .read(&mut left_buffer)
            .map_err(|source| io_error("compare file", Path::new("<left>"), source))?;
        let right_count = right
            .read(&mut right_buffer)
            .map_err(|source| io_error("compare file", Path::new("<right>"), source))?;
        if left_count != right_count || left_buffer[..left_count] != right_buffer[..right_count] {
            return Ok(false);
        }
        if left_count == 0 {
            return Ok(true);
        }
    }
}

pub fn same_file_identity(left: &Path, right: &Path) -> Result<bool, FsError> {
    same_file::is_same_file(left, right)
        .map_err(|source| io_error("compare file identity", left, source))
}

pub fn remove_verified_source(source: &Path, destination: &Path) -> Result<(), FsError> {
    if !files_equal(source, destination)? {
        return Err(FsError::VerificationFailed {
            source_path: source.to_path_buf(),
            temporary_path: destination.to_path_buf(),
        });
    }
    fs::remove_file(source).map_err(|error| FsError::SourceRemovalFailed {
        source_path: source.to_path_buf(),
        destination_path: destination.to_path_buf(),
        source: error,
    })?;
    sync_source_parent(source);
    Ok(())
}

pub fn publish_verified_temporary(temporary: &Path, destination: &Path) -> Result<(), FsError> {
    let Some(parent) = destination.parent() else {
        return Err(FsError::InvalidDestination(destination.to_path_buf()));
    };
    reject_destination_symlinks(parent)?;
    fs::hard_link(temporary, destination).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            FsError::Collision(destination.to_path_buf())
        } else {
            staged_error("publish recovered temporary file", destination, source)
        }
    })?;
    File::open(destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::PublishedSyncFailed {
            source_path: temporary.to_path_buf(),
            destination_path: destination.to_path_buf(),
            source,
        })?;
    sync_directory(parent);
    Ok(())
}

fn validate_source(path: &Path) -> Result<fs::Metadata, FsError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            FsError::SourceMissing(path.to_path_buf())
        } else {
            io_error("read source metadata", path, source)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FsError::SymbolicLink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(FsError::SourceNotRegular(path.to_path_buf()));
    }
    Ok(metadata)
}

fn prepare_destination_parent(destination: &Path) -> Result<&Path, FsError> {
    let Some(parent) = destination.parent() else {
        return Err(FsError::InvalidDestination(destination.to_path_buf()));
    };
    reject_destination_symlinks(parent)?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create destination directory", parent, source))?;
    reject_destination_symlinks(parent)?;
    Ok(parent)
}

fn temporary_path(destination: &Path, tag: &str) -> PathBuf {
    let tag: String = tag
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let filename = destination
        .file_name()
        .map_or_else(|| "file".into(), |name| name.to_string_lossy());
    destination.with_file_name(format!(".{filename}.sorter-{tag}.partial"))
}

fn resolve_destination(path: &Path, policy: CollisionPolicy) -> Result<PathBuf, FsError> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }
    match policy {
        CollisionPolicy::Skip => Ok(path.to_path_buf()),
        CollisionPolicy::Error => Err(FsError::Collision(path.to_path_buf())),
        CollisionPolicy::RenameWithSuffix => {
            for index in 1_u32..=u32::MAX {
                let candidate = suffixed_path(path, index);
                if !candidate.exists() {
                    return Ok(candidate);
                }
            }
            unreachable!("u32 suffix space exhausted")
        }
    }
}

fn suffixed_path(path: &Path, index: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "file".into(), |value| value.to_string_lossy());
    let extension = path.extension().map(|value| value.to_string_lossy());
    let filename = extension.map_or_else(
        || format!("{stem} ({index})"),
        |extension| format!("{stem} ({index}).{extension}"),
    );
    path.with_file_name(filename)
}

fn ensure_source_unchanged(source: &Path, expected: &FileFingerprint) -> Result<(), FsError> {
    let current = validate_source(source)?;
    if fingerprint(&current) == *expected {
        Ok(())
    } else {
        Err(FsError::SourceChanged(source.to_path_buf()))
    }
}

fn ensure_file_unchanged(
    file: &File,
    source_path: &Path,
    expected: &FileFingerprint,
) -> Result<(), FsError> {
    let current = file
        .metadata()
        .map_err(|source| staged_error("read open source metadata", source_path, source))?;
    if fingerprint(&current) == *expected {
        Ok(())
    } else {
        Err(FsError::SourceChanged(source_path.to_path_buf()))
    }
}

fn fingerprint(metadata: &fs::Metadata) -> FileFingerprint {
    let modified_unix_ns = metadata.modified().ok().and_then(|time| {
        time.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i128::try_from(duration.as_nanos()).ok())
    });
    let (device, file_id) = platform_identity(metadata);
    FileFingerprint {
        size: metadata.len(),
        modified_unix_ns,
        device,
        file_id,
    }
}

#[cfg(unix)]
fn platform_identity(metadata: &fs::Metadata) -> (Option<u64>, Option<u64>) {
    use std::os::unix::fs::MetadataExt;
    (Some(metadata.dev()), Some(metadata.ino()))
}

#[cfg(not(unix))]
fn platform_identity(_metadata: &fs::Metadata) -> (Option<u64>, Option<u64>) {
    (None, None)
}

fn record(hooks: &mut impl TransferHooks, event: TransferEvent) -> Result<(), FsError> {
    let point = event.name();
    hooks
        .record(event)
        .map_err(|message| FsError::HookFailed { point, message })
}

fn checkpoint(hooks: &mut impl TransferHooks, point: FaultPoint) -> Result<(), FsError> {
    hooks
        .checkpoint(point)
        .map_err(|message| FsError::HookFailed {
            point: point.name(),
            message,
        })
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> FsError {
    FsError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn staged_error(operation: &'static str, path: &Path, source: io::Error) -> FsError {
    FsError::StagedTransfer {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn reject_destination_symlinks(path: &Path) -> Result<(), FsError> {
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(FsError::SymbolicLinkDestination(ancestor.to_path_buf()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(io_error("validate destination directory", ancestor, source));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn is_cross_device(error: &io::Error) -> bool {
    error.raw_os_error() == Some(18)
}

#[cfg(windows)]
fn is_cross_device(error: &io::Error) -> bool {
    error.raw_os_error() == Some(17)
}

#[cfg(not(any(unix, windows)))]
fn is_cross_device(_error: &io::Error) -> bool {
    false
}

fn sync_source_parent(source: &Path) {
    if let Some(parent) = source.parent() {
        sync_directory(parent);
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) {
    if let Err(error) = File::open(path).and_then(|directory| directory.sync_all()) {
        warn!(path = %path.display(), %error, "could not synchronize directory metadata");
    }
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn create_plan(root: &Path) -> PlannedMove {
        let source = root.join("incoming/report.txt");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        File::create(&source)
            .unwrap()
            .write_all(b"safe data")
            .unwrap();
        PlannedMove {
            source,
            destination: root.join("sorted/report.txt"),
            rule_id: "documents".into(),
            priority: 10,
        }
    }

    #[derive(Default)]
    struct RecordingHooks {
        events: Vec<&'static str>,
        fail_at: Option<FaultPoint>,
        fail_record: Option<&'static str>,
    }

    impl TransferHooks for RecordingHooks {
        fn record(&mut self, event: TransferEvent) -> Result<(), String> {
            if self.fail_record == Some(event.name()) {
                return Err(format!("injected journal failure at {}", event.name()));
            }
            self.events.push(event.name());
            Ok(())
        }

        fn checkpoint(&mut self, point: FaultPoint) -> Result<(), String> {
            if self.fail_at == Some(point) {
                Err(format!("injected failure at {}", point.name()))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn moves_without_overwriting() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let result = execute_move(&plan, CollisionPolicy::Error).unwrap();
        assert!(matches!(result, MoveOutcome::Moved { .. }));
        assert!(!plan.source.exists());
        assert_eq!(fs::read(&plan.destination).unwrap(), b"safe data");
    }

    #[test]
    fn staged_copy_is_verified_and_removes_source_last() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks::default();
        let outcome =
            execute_move_with_hooks(&plan, CollisionPolicy::Error, "test", true, &mut hooks)
                .unwrap();
        assert!(matches!(
            outcome,
            MoveOutcome::Moved {
                kind: TransferKind::StagedCopy,
                ..
            }
        ));
        assert_eq!(
            hooks.events,
            [
                "prepared",
                "copied",
                "verified",
                "published",
                "source_removed"
            ]
        );
        assert!(!plan.source.exists());
        assert_eq!(fs::read(&plan.destination).unwrap(), b"safe data");
    }

    #[cfg(unix)]
    #[test]
    fn crosses_filesystems_when_distinct_mounts_are_available() {
        use std::os::unix::fs::MetadataExt;

        if !Path::new("/dev/shm").is_dir() {
            return;
        }
        let source_root = tempdir().unwrap();
        let Ok(destination_root) = tempfile::tempdir_in("/dev/shm") else {
            return;
        };
        if fs::metadata(source_root.path()).unwrap().dev()
            == fs::metadata(destination_root.path()).unwrap().dev()
        {
            return;
        }
        let mut plan = create_plan(source_root.path());
        plan.destination = destination_root.path().join("report.txt");
        let outcome = execute_move(&plan, CollisionPolicy::Error).unwrap();
        assert!(matches!(
            outcome,
            MoveOutcome::Moved {
                kind: TransferKind::StagedCopy,
                ..
            }
        ));
        assert!(!plan.source.exists());
        assert_eq!(fs::read(&plan.destination).unwrap(), b"safe data");
    }

    #[test]
    fn interruption_during_copy_preserves_source_and_partial() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks {
            fail_at: Some(FaultPoint::DuringCopy),
            ..RecordingHooks::default()
        };
        assert!(execute_move_with_hooks(
            &plan,
            CollisionPolicy::Error,
            "during-copy",
            true,
            &mut hooks
        )
        .is_err());
        assert!(plan.source.exists());
        assert!(!plan.destination.exists());
        assert!(temporary_path(&plan.destination, "during-copy").exists());
    }

    #[test]
    fn interruptions_before_publication_never_remove_source() {
        for point in [
            FaultPoint::BeforeCopy,
            FaultPoint::AfterCopy,
            FaultPoint::AfterVerification,
            FaultPoint::BeforePublication,
        ] {
            let root = tempdir().unwrap();
            let plan = create_plan(root.path());
            let mut hooks = RecordingHooks {
                fail_at: Some(point),
                ..RecordingHooks::default()
            };
            assert!(execute_move_with_hooks(
                &plan,
                CollisionPolicy::Error,
                point.name(),
                true,
                &mut hooks
            )
            .is_err());
            assert!(plan.source.exists(), "source lost at {}", point.name());
            assert!(
                !plan.destination.exists(),
                "destination published at {}",
                point.name()
            );
        }
    }

    #[test]
    fn journal_failure_after_publication_preserves_all_data() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks {
            fail_record: Some("published"),
            ..RecordingHooks::default()
        };
        assert!(execute_move_with_hooks(
            &plan,
            CollisionPolicy::Error,
            "journal-published",
            true,
            &mut hooks
        )
        .is_err());
        assert!(plan.source.exists());
        assert!(plan.destination.exists());
        assert!(temporary_path(&plan.destination, "journal-published").exists());
        assert_eq!(fs::read(&plan.destination).unwrap(), b"safe data");
    }

    #[test]
    fn interruption_before_source_deletion_preserves_both_names() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks {
            fail_at: Some(FaultPoint::BeforeSourceDeletion),
            ..RecordingHooks::default()
        };
        assert!(execute_move_with_hooks(
            &plan,
            CollisionPolicy::Error,
            "before-delete",
            true,
            &mut hooks
        )
        .is_err());
        assert!(plan.source.exists());
        assert!(plan.destination.exists());
    }

    #[test]
    fn interruption_after_publication_preserves_both_files() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks {
            fail_at: Some(FaultPoint::AfterPublication),
            ..RecordingHooks::default()
        };
        assert!(execute_move_with_hooks(
            &plan,
            CollisionPolicy::Error,
            "published",
            true,
            &mut hooks
        )
        .is_err());
        assert!(plan.source.exists());
        assert!(plan.destination.exists());
        assert_eq!(fs::read(&plan.destination).unwrap(), b"safe data");
    }

    #[test]
    fn interruption_after_source_deletion_is_recoverable_from_destination() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = RecordingHooks {
            fail_at: Some(FaultPoint::AfterSourceDeletion),
            ..RecordingHooks::default()
        };
        assert!(execute_move_with_hooks(
            &plan,
            CollisionPolicy::Error,
            "source-removed",
            true,
            &mut hooks
        )
        .is_err());
        assert!(!plan.source.exists());
        assert!(plan.destination.exists());
    }

    #[test]
    fn skips_collision_and_preserves_both_files() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        fs::create_dir_all(plan.destination.parent().unwrap()).unwrap();
        fs::write(&plan.destination, b"existing").unwrap();
        let result = execute_move(&plan, CollisionPolicy::Skip).unwrap();
        assert!(matches!(result, MoveOutcome::SkippedCollision { .. }));
        assert_eq!(fs::read(&plan.source).unwrap(), b"safe data");
        assert_eq!(fs::read(&plan.destination).unwrap(), b"existing");
    }

    #[test]
    fn suffix_policy_preserves_existing_destination() {
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        fs::create_dir_all(plan.destination.parent().unwrap()).unwrap();
        fs::write(&plan.destination, b"existing").unwrap();
        let result = execute_move(&plan, CollisionPolicy::RenameWithSuffix).unwrap();
        let MoveOutcome::Moved { destination, .. } = result else {
            panic!("expected move")
        };
        assert_eq!(destination.file_name().unwrap(), "report (1).txt");
        assert_eq!(fs::read(&plan.destination).unwrap(), b"existing");
    }

    #[test]
    fn staged_suffix_policy_handles_publication_race() {
        struct CollisionHook {
            destination: PathBuf,
        }

        impl TransferHooks for CollisionHook {
            fn record(&mut self, _event: TransferEvent) -> Result<(), String> {
                Ok(())
            }

            fn checkpoint(&mut self, point: FaultPoint) -> Result<(), String> {
                if point == FaultPoint::BeforePublication {
                    fs::write(&self.destination, b"raced").map_err(|error| error.to_string())?;
                }
                Ok(())
            }
        }

        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let mut hooks = CollisionHook {
            destination: plan.destination.clone(),
        };
        let outcome = execute_move_with_hooks(
            &plan,
            CollisionPolicy::RenameWithSuffix,
            "collision-race",
            true,
            &mut hooks,
        )
        .unwrap();
        let MoveOutcome::Moved { destination, .. } = outcome else {
            panic!("expected move")
        };
        assert_eq!(destination.file_name().unwrap(), "report (1).txt");
        assert_eq!(fs::read(&plan.destination).unwrap(), b"raced");
        assert_eq!(fs::read(destination).unwrap(), b"safe data");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_links() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let link = root.path().join("incoming/link.txt");
        symlink(&plan.source, &link).unwrap();
        let link_plan = PlannedMove {
            source: link,
            ..plan
        };
        assert!(matches!(
            execute_move(&link_plan, CollisionPolicy::Error),
            Err(FsError::SymbolicLink(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_destination_directory() {
        use std::os::unix::fs::symlink;
        let root = tempdir().unwrap();
        let plan = create_plan(root.path());
        let real_destination = root.path().join("real-sorted");
        fs::create_dir(&real_destination).unwrap();
        symlink(&real_destination, root.path().join("sorted")).unwrap();
        assert!(matches!(
            execute_move(&plan, CollisionPolicy::Error),
            Err(FsError::SymbolicLinkDestination(_))
        ));
        assert!(plan.source.exists());
    }
}
