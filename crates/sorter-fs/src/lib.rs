//! Conservative same-filesystem file operations.

use serde::{Deserialize, Serialize};
use sorter_core::PlannedMove;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::warn;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CollisionPolicy {
    #[default]
    Skip,
    RenameWithSuffix,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoveOutcome {
    Moved { destination: PathBuf },
    SkippedCollision { destination: PathBuf },
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
    #[error("destination has no parent: {0}")]
    InvalidDestination(PathBuf),
    #[error("source and destination refer to the same path: {0}")]
    SamePath(PathBuf),
    #[error("cross-filesystem transfer is not available in Phase 2: {source_path} -> {destination_path}")]
    CrossFilesystem {
        source_path: PathBuf,
        destination_path: PathBuf,
    },
    #[error("filesystem operation {operation} failed for {path}: {source}")]
    Io {
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
}

pub fn execute_move(
    planned: &PlannedMove,
    collision_policy: CollisionPolicy,
) -> Result<MoveOutcome, FsError> {
    let metadata = fs::symlink_metadata(&planned.source).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            FsError::SourceMissing(planned.source.clone())
        } else {
            io_error("read source metadata", &planned.source, source)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FsError::SymbolicLink(planned.source.clone()));
    }
    if !metadata.is_file() {
        return Err(FsError::SourceNotRegular(planned.source.clone()));
    }
    if planned.source == planned.destination {
        return Err(FsError::SamePath(planned.source.clone()));
    }

    let destination = resolve_destination(&planned.destination, collision_policy)?;
    let Some(parent) = destination.parent() else {
        return Err(FsError::InvalidDestination(destination));
    };
    reject_destination_symlinks(parent)?;
    fs::create_dir_all(parent)
        .map_err(|source| io_error("create destination directory", parent, source))?;
    reject_destination_symlinks(parent)?;

    // hard_link is an atomic no-clobber publication on a single filesystem.
    // The source remains intact until the new directory entry is durable.
    match fs::hard_link(&planned.source, &destination) {
        Ok(()) => {}
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            if collision_policy == CollisionPolicy::RenameWithSuffix {
                return execute_move(planned, collision_policy);
            }
            return handle_late_collision(destination, collision_policy);
        }
        Err(source) if is_cross_device(&source) => {
            return Err(FsError::CrossFilesystem {
                source_path: planned.source.clone(),
                destination_path: destination,
            });
        }
        Err(source) => return Err(io_error("publish destination", &destination, source)),
    }

    File::open(&destination)
        .and_then(|file| file.sync_all())
        .map_err(|source| FsError::PublishedSyncFailed {
            source_path: planned.source.clone(),
            destination_path: destination.clone(),
            source,
        })?;
    sync_directory(parent);

    fs::remove_file(&planned.source).map_err(|source| FsError::SourceRemovalFailed {
        source_path: planned.source.clone(),
        destination_path: destination.clone(),
        source,
    })?;
    if let Some(source_parent) = planned.source.parent() {
        sync_directory(source_parent);
    }

    Ok(MoveOutcome::Moved { destination })
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

fn handle_late_collision(
    destination: PathBuf,
    policy: CollisionPolicy,
) -> Result<MoveOutcome, FsError> {
    match policy {
        CollisionPolicy::Skip => Ok(MoveOutcome::SkippedCollision { destination }),
        CollisionPolicy::Error | CollisionPolicy::RenameWithSuffix => {
            Err(FsError::Collision(destination))
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

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> FsError {
    FsError::Io {
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

#[cfg(unix)]
fn sync_directory(path: &Path) {
    if let Err(error) = File::open(path).and_then(|directory| directory.sync_all()) {
        // Some filesystems do not allow directory fsync. The data file itself
        // has already been synchronized, so report this durability limitation.
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
        let MoveOutcome::Moved { destination } = result else {
            panic!("expected move")
        };
        assert_eq!(destination.file_name().unwrap(), "report (1).txt");
        assert_eq!(fs::read(&plan.destination).unwrap(), b"existing");
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
