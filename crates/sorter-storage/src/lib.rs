//! TOML configuration and `SQLite` operation history.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sorter_core::{MatchEngine, Rule};
use sorter_fs::{CollisionPolicy, FileFingerprint, TransferKind};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("could not read configuration {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not parse configuration {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not serialize configuration: {0}")]
    SerializeConfig(toml::ser::Error),
    #[error("could not write configuration {path}: {source}")]
    WriteConfig {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("unsupported configuration version {found}; this binary supports {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("duplicate directory id: {0}")]
    DuplicateDirectoryId(String),
    #[error("directory id must not be empty")]
    EmptyDirectoryId,
    #[error("directory {directory_id} references unknown rule {rule_id}")]
    UnknownRule {
        directory_id: String,
        rule_id: String,
    },
    #[error("directory {directory_id} has no rules")]
    DirectoryWithoutRules { directory_id: String },
    #[error("directory {directory_id} has an empty source path")]
    EmptyDirectoryPath { directory_id: String },
    #[error("scheduled directory {directory_id} is missing its cron expression")]
    MissingSchedule { directory_id: String },
    #[error("directory {directory_id} has an invalid cron expression {schedule:?}: {reason}")]
    InvalidSchedule {
        directory_id: String,
        schedule: String,
        reason: String,
    },
    #[error("invalid automation setting {name}: {reason}")]
    InvalidAutomationSetting { name: &'static str, reason: String },
    #[error("invalid temporary-file glob {pattern:?}: {reason}")]
    InvalidTemporaryGlob { pattern: String, reason: String },
    #[error(
        "destination for rule {rule_id} would feed source directory {directory_id}: {destination}"
    )]
    RecursiveDestinationLoop {
        directory_id: String,
        rule_id: String,
        destination: PathBuf,
    },
    #[error(transparent)]
    Core(#[from] sorter_core::CoreError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("another unfinished operation already owns source file {0}")]
    SourceAlreadyActive(PathBuf),
    #[error("operation {0} was not found")]
    OperationNotFound(i64),
    #[error("invalid operation state stored in the database: {0}")]
    InvalidOperationState(String),
    #[error("invalid transfer kind stored in the database: {0}")]
    InvalidTransferKind(String),
    #[error("invalid numeric recovery metadata stored in the database: {0}")]
    InvalidRecoveryMetadata(String),
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub directories: Vec<DirectoryConfig>,
}

const fn current_version() -> u32 {
    CURRENT_CONFIG_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub collision_policy: CollisionPolicy,
    pub watch_debounce_ms: u64,
    pub stabilization_delay_ms: u64,
    pub reconciliation_interval_seconds: u64,
    pub event_queue_capacity: usize,
    pub max_concurrent_operations: usize,
    pub temporary_file_globs: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            collision_policy: CollisionPolicy::Skip,
            watch_debounce_ms: 500,
            stabilization_delay_ms: 2_000,
            reconciliation_interval_seconds: 300,
            event_queue_capacity: 1_024,
            max_concurrent_operations: 2,
            temporary_file_globs: vec![
                "*.crdownload".into(),
                "*.download".into(),
                "*.part".into(),
                "*.partial".into(),
                "*.tmp".into(),
                "~$*".into(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    #[default]
    Once,
    Watch,
    Scheduled,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleTimezone {
    #[default]
    Local,
    Utc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryConfig {
    pub id: String,
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub schedule_timezone: ScheduleTimezone,
    pub rules: Vec<String>,
}

const fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, StorageError> {
        let content = fs::read_to_string(path).map_err(|source| StorageError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self =
            toml::from_str(&content).map_err(|source| StorageError::ParseConfig {
                path: path.to_path_buf(),
                source,
            })?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<(), StorageError> {
        self.validate()?;
        let content = toml::to_string_pretty(self).map_err(StorageError::SerializeConfig)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| StorageError::WriteConfig {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(path, content).map_err(|source| StorageError::WriteConfig {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        if self.version != CURRENT_CONFIG_VERSION {
            return Err(StorageError::UnsupportedVersion {
                found: self.version,
                supported: CURRENT_CONFIG_VERSION,
            });
        }
        MatchEngine::compile(&self.rules)?;
        self.settings.validate_automation()?;
        let rules_by_id: HashMap<_, _> = self
            .rules
            .iter()
            .map(|rule| (rule.id.as_str(), rule))
            .collect();
        let mut directory_ids = HashSet::new();
        for directory in &self.directories {
            if directory.id.trim().is_empty() {
                return Err(StorageError::EmptyDirectoryId);
            }
            if !directory_ids.insert(directory.id.as_str()) {
                return Err(StorageError::DuplicateDirectoryId(directory.id.clone()));
            }
            if directory.path.as_os_str().is_empty() {
                return Err(StorageError::EmptyDirectoryPath {
                    directory_id: directory.id.clone(),
                });
            }
            if directory.rules.is_empty() {
                return Err(StorageError::DirectoryWithoutRules {
                    directory_id: directory.id.clone(),
                });
            }
            if directory.mode == ExecutionMode::Scheduled {
                let schedule =
                    directory
                        .schedule
                        .as_deref()
                        .ok_or_else(|| StorageError::MissingSchedule {
                            directory_id: directory.id.clone(),
                        })?;
                cron::Schedule::from_str(schedule).map_err(|error| {
                    StorageError::InvalidSchedule {
                        directory_id: directory.id.clone(),
                        schedule: schedule.to_owned(),
                        reason: error.to_string(),
                    }
                })?;
            }
            for rule_id in &directory.rules {
                let rule =
                    rules_by_id
                        .get(rule_id.as_str())
                        .ok_or_else(|| StorageError::UnknownRule {
                            directory_id: directory.id.clone(),
                            rule_id: rule_id.clone(),
                        })?;
                if rule.destination == directory.path
                    || (directory.recursive && rule.destination.starts_with(&directory.path))
                {
                    return Err(StorageError::RecursiveDestinationLoop {
                        directory_id: directory.id.clone(),
                        rule_id: rule.id.clone(),
                        destination: rule.destination.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn rules_for(&self, directory: &DirectoryConfig) -> Vec<Rule> {
        let ids: HashSet<_> = directory.rules.iter().map(String::as_str).collect();
        self.rules
            .iter()
            .filter(|rule| ids.contains(rule.id.as_str()))
            .cloned()
            .collect()
    }
}

impl Settings {
    fn validate_automation(&self) -> Result<(), StorageError> {
        for (name, value) in [
            ("watch_debounce_ms", self.watch_debounce_ms),
            ("stabilization_delay_ms", self.stabilization_delay_ms),
            (
                "reconciliation_interval_seconds",
                self.reconciliation_interval_seconds,
            ),
        ] {
            if value == 0 {
                return Err(StorageError::InvalidAutomationSetting {
                    name,
                    reason: "must be greater than zero".into(),
                });
            }
        }
        for (name, value) in [
            ("event_queue_capacity", self.event_queue_capacity),
            ("max_concurrent_operations", self.max_concurrent_operations),
        ] {
            if value == 0 {
                return Err(StorageError::InvalidAutomationSetting {
                    name,
                    reason: "must be greater than zero".into(),
                });
            }
        }
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in &self.temporary_file_globs {
            let glob = globset::Glob::new(pattern).map_err(|error| {
                StorageError::InvalidTemporaryGlob {
                    pattern: pattern.clone(),
                    reason: error.to_string(),
                }
            })?;
            builder.add(glob);
        }
        builder
            .build()
            .map_err(|error| StorageError::InvalidTemporaryGlob {
                pattern: "<set>".into(),
                reason: error.to_string(),
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationState {
    Planned,
    InProgress,
    Copied,
    Verified,
    Published,
    Completed,
    Failed,
    RecoveryRequired,
    Skipped,
}

impl OperationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "PLANNED",
            Self::InProgress => "IN_PROGRESS",
            Self::Copied => "COPIED",
            Self::Verified => "VERIFIED",
            Self::Published => "PUBLISHED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::Skipped => "SKIPPED",
        }
    }

    pub fn parse(value: &str) -> Result<Self, StorageError> {
        match value {
            "PLANNED" => Ok(Self::Planned),
            "IN_PROGRESS" => Ok(Self::InProgress),
            "COPIED" => Ok(Self::Copied),
            "VERIFIED" => Ok(Self::Verified),
            "PUBLISHED" => Ok(Self::Published),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "RECOVERY_REQUIRED" => Ok(Self::RecoveryRequired),
            "SKIPPED" => Ok(Self::Skipped),
            other => Err(StorageError::InvalidOperationState(other.to_owned())),
        }
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Skipped)
    }
}

#[derive(Debug, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub source: String,
    pub destination: String,
    pub temporary_path: Option<String>,
    pub rule_id: String,
    pub state: String,
    pub transfer_kind: Option<String>,
    pub error: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone)]
pub struct OperationRecord {
    pub id: i64,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub temporary_path: Option<PathBuf>,
    pub rule_id: String,
    pub state: OperationState,
    pub transfer_kind: Option<TransferKind>,
    pub source_fingerprint: Option<FileFingerprint>,
    pub error: Option<String>,
}

pub struct HistoryStore {
    connection: Connection,
}

impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| StorageError::WriteConfig {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS operations (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                source_raw BLOB,
                destination TEXT NOT NULL,
                destination_raw BLOB,
                temporary_path TEXT,
                temporary_path_raw BLOB,
                rule_id TEXT NOT NULL,
                state TEXT NOT NULL,
                transfer_kind TEXT,
                source_size TEXT,
                source_modified_ns TEXT,
                source_device TEXT,
                source_file_id TEXT,
                error TEXT,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS operations_updated_idx
                ON operations(updated_at_unix_ms DESC);",
        )?;
        migrate_operation_columns(&connection)?;
        connection.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS operations_active_source_idx
                 ON operations(source_raw)
                 WHERE source_raw IS NOT NULL
                   AND state IN (
                       'PLANNED', 'IN_PROGRESS', 'COPIED', 'VERIFIED',
                       'PUBLISHED', 'RECOVERY_REQUIRED'
                   );",
        )?;
        Ok(Self { connection })
    }

    pub fn create_operation(
        &self,
        source: &Path,
        destination: &Path,
        rule_id: &str,
    ) -> Result<i64, StorageError> {
        let now = timestamp_ms()?;
        let result = self.connection.execute(
            "INSERT INTO operations
             (source, source_raw, destination, destination_raw, rule_id, state,
              created_at_unix_ms, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                source.to_string_lossy(),
                path_to_blob(source),
                destination.to_string_lossy(),
                path_to_blob(destination),
                rule_id,
                OperationState::Planned.as_str(),
                now
            ],
        );
        if let Err(error) = result {
            if matches!(
                error,
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error {
                        code: rusqlite::ErrorCode::ConstraintViolation,
                        ..
                    },
                    _
                )
            ) {
                return Err(StorageError::SourceAlreadyActive(source.to_path_buf()));
            }
            return Err(error.into());
        }
        Ok(self.connection.last_insert_rowid())
    }

    pub fn update_operation(
        &self,
        id: i64,
        state: OperationState,
        error: Option<&str>,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE operations SET state = ?1, error = ?2, updated_at_unix_ms = ?3 WHERE id = ?4",
            params![state.as_str(), error, timestamp_ms()?, id],
        )?;
        Ok(())
    }

    pub fn update_destination(&self, id: i64, destination: &Path) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE operations
             SET destination = ?1, destination_raw = ?2, updated_at_unix_ms = ?3
             WHERE id = ?4",
            params![
                destination.to_string_lossy(),
                path_to_blob(destination),
                timestamp_ms()?,
                id
            ],
        )?;
        Ok(())
    }

    pub fn record_prepared(
        &self,
        id: i64,
        destination: &Path,
        temporary_path: Option<&Path>,
        kind: TransferKind,
        source: &FileFingerprint,
    ) -> Result<(), StorageError> {
        self.connection.execute(
            "UPDATE operations
             SET destination = ?1, destination_raw = ?2,
                 temporary_path = ?3, temporary_path_raw = ?4,
                 transfer_kind = ?5, source_size = ?6,
                 source_modified_ns = ?7, source_device = ?8, source_file_id = ?9,
                 state = ?10, error = NULL, updated_at_unix_ms = ?11
             WHERE id = ?12",
            params![
                destination.to_string_lossy(),
                path_to_blob(destination),
                temporary_path.map(|path| path.to_string_lossy().into_owned()),
                temporary_path.map(path_to_blob),
                transfer_kind_as_str(kind),
                source.size.to_string(),
                source.modified_unix_ns.map(|value| value.to_string()),
                source.device.map(|value| value.to_string()),
                source.file_id.map(|value| value.to_string()),
                OperationState::InProgress.as_str(),
                timestamp_ms()?,
                id,
            ],
        )?;
        Ok(())
    }

    pub fn operation(&self, id: i64) -> Result<OperationRecord, StorageError> {
        let query_result = self.connection.query_row(
            "SELECT id, source, source_raw, destination, destination_raw,
                    temporary_path, temporary_path_raw, rule_id, state, transfer_kind,
                    source_size, source_modified_ns, source_device, source_file_id, error
             FROM operations WHERE id = ?1",
            [id],
            operation_row,
        );
        match query_result {
            Ok(record) => record.try_into(),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(StorageError::OperationNotFound(id)),
            Err(error) => Err(error.into()),
        }
    }

    pub fn recoverable(
        &self,
        operation_id: Option<i64>,
    ) -> Result<Vec<OperationRecord>, StorageError> {
        let mut result = Vec::new();
        if let Some(id) = operation_id {
            let operation = self.operation(id)?;
            if !operation.state.is_terminal() {
                result.push(operation);
            }
            return Ok(result);
        }
        let mut statement = self.connection.prepare(
            "SELECT id, source, source_raw, destination, destination_raw,
                    temporary_path, temporary_path_raw, rule_id, state, transfer_kind,
                    source_size, source_modified_ns, source_device, source_file_id, error
             FROM operations
             WHERE state IN (
                 'PLANNED', 'IN_PROGRESS', 'COPIED', 'VERIFIED',
                 'PUBLISHED', 'RECOVERY_REQUIRED'
             )
             ORDER BY id",
        )?;
        let rows = statement.query_map([], operation_row)?;
        for row in rows {
            result.push(row?.try_into()?);
        }
        Ok(result)
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<HistoryEntry>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, source, destination, temporary_path, rule_id, state,
                    transfer_kind, error,
                    created_at_unix_ms, updated_at_unix_ms
             FROM operations ORDER BY updated_at_unix_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                source: row.get(1)?,
                destination: row.get(2)?,
                temporary_path: row.get(3)?,
                rule_id: row.get(4)?,
                state: row.get(5)?,
                transfer_kind: row.get(6)?,
                error: row.get(7)?,
                created_at_unix_ms: row.get(8)?,
                updated_at_unix_ms: row.get(9)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[derive(Debug)]
struct RawOperationRecord {
    id: i64,
    source: String,
    source_raw: Option<Vec<u8>>,
    destination: String,
    destination_raw: Option<Vec<u8>>,
    temporary_path: Option<String>,
    temporary_path_raw: Option<Vec<u8>>,
    rule_id: String,
    state: String,
    transfer_kind: Option<String>,
    source_size: Option<String>,
    source_modified_ns: Option<String>,
    source_device: Option<String>,
    source_file_id: Option<String>,
    error: Option<String>,
}

fn operation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawOperationRecord> {
    Ok(RawOperationRecord {
        id: row.get(0)?,
        source: row.get(1)?,
        source_raw: row.get(2)?,
        destination: row.get(3)?,
        destination_raw: row.get(4)?,
        temporary_path: row.get(5)?,
        temporary_path_raw: row.get(6)?,
        rule_id: row.get(7)?,
        state: row.get(8)?,
        transfer_kind: row.get(9)?,
        source_size: row.get(10)?,
        source_modified_ns: row.get(11)?,
        source_device: row.get(12)?,
        source_file_id: row.get(13)?,
        error: row.get(14)?,
    })
}

impl TryFrom<RawOperationRecord> for OperationRecord {
    type Error = StorageError;

    fn try_from(raw: RawOperationRecord) -> Result<Self, Self::Error> {
        let source_fingerprint = raw
            .source_size
            .map(|size| {
                Ok::<_, StorageError>(FileFingerprint {
                    size: parse_number(&size)?,
                    modified_unix_ns: raw
                        .source_modified_ns
                        .as_deref()
                        .map(parse_number)
                        .transpose()?,
                    device: raw.source_device.as_deref().map(parse_number).transpose()?,
                    file_id: raw
                        .source_file_id
                        .as_deref()
                        .map(parse_number)
                        .transpose()?,
                })
            })
            .transpose()?;
        Ok(Self {
            id: raw.id,
            source: raw
                .source_raw
                .map_or_else(|| PathBuf::from(raw.source), blob_to_path),
            destination: raw
                .destination_raw
                .map_or_else(|| PathBuf::from(raw.destination), blob_to_path),
            temporary_path: raw
                .temporary_path_raw
                .map(blob_to_path)
                .or_else(|| raw.temporary_path.map(PathBuf::from)),
            rule_id: raw.rule_id,
            state: OperationState::parse(&raw.state)?,
            transfer_kind: raw
                .transfer_kind
                .as_deref()
                .map(parse_transfer_kind)
                .transpose()?,
            source_fingerprint,
            error: raw.error,
        })
    }
}

fn parse_number<T: FromStr>(value: &str) -> Result<T, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidRecoveryMetadata(value.to_owned()))
}

const fn transfer_kind_as_str(kind: TransferKind) -> &'static str {
    match kind {
        TransferKind::HardLink => "HARD_LINK",
        TransferKind::StagedCopy => "STAGED_COPY",
    }
}

fn parse_transfer_kind(value: &str) -> Result<TransferKind, StorageError> {
    match value {
        "HARD_LINK" => Ok(TransferKind::HardLink),
        "STAGED_COPY" => Ok(TransferKind::StagedCopy),
        other => Err(StorageError::InvalidTransferKind(other.to_owned())),
    }
}

fn migrate_operation_columns(connection: &Connection) -> Result<(), StorageError> {
    let mut statement = connection.prepare("PRAGMA table_info(operations)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<HashSet<_>, _>>()?;
    drop(statement);
    let additions = [
        ("source_raw", "BLOB"),
        ("destination_raw", "BLOB"),
        ("temporary_path", "TEXT"),
        ("temporary_path_raw", "BLOB"),
        ("transfer_kind", "TEXT"),
        ("source_size", "TEXT"),
        ("source_modified_ns", "TEXT"),
        ("source_device", "TEXT"),
        ("source_file_id", "TEXT"),
    ];
    for (column, data_type) in additions {
        if !columns.contains(column) {
            connection.execute(
                &format!("ALTER TABLE operations ADD COLUMN {column} {data_type}"),
                [],
            )?;
        }
    }
    let mut statement = connection.prepare(
        "SELECT id, source, destination, temporary_path
         FROM operations
         WHERE source_raw IS NULL OR destination_raw IS NULL",
    )?;
    let legacy_rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (id, source, destination, temporary_path) in legacy_rows {
        connection.execute(
            "UPDATE operations
             SET source_raw = ?1, destination_raw = ?2, temporary_path_raw = ?3
             WHERE id = ?4",
            params![
                path_to_blob(Path::new(&source)),
                path_to_blob(Path::new(&destination)),
                temporary_path.as_deref().map(Path::new).map(path_to_blob),
                id
            ],
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn path_to_blob(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(unix)]
fn blob_to_path(value: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(value).into()
}

#[cfg(windows)]
fn path_to_blob(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(windows)]
fn blob_to_path(value: Vec<u8>) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let units = value
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]));
    std::ffi::OsString::from_wide(&units.collect::<Vec<_>>()).into()
}

#[cfg(not(any(unix, windows)))]
fn path_to_blob(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

#[cfg(not(any(unix, windows)))]
fn blob_to_path(value: Vec<u8>) -> PathBuf {
    String::from_utf8_lossy(&value).into_owned().into()
}

fn timestamp_ms() -> Result<i64, StorageError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StorageError::InvalidClock)?;
    i64::try_from(duration.as_millis()).map_err(|_| StorageError::InvalidClock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorter_core::MatchSpec;
    use tempfile::tempdir;

    fn config() -> Config {
        Config {
            version: CURRENT_CONFIG_VERSION,
            settings: Settings::default(),
            rules: vec![Rule {
                id: "documents".into(),
                enabled: true,
                priority: 10,
                destination: "/sorted".into(),
                match_spec: MatchSpec {
                    extensions: vec!["pdf".into()],
                    ..MatchSpec::default()
                },
            }],
            directories: vec![DirectoryConfig {
                id: "downloads".into(),
                path: "/incoming".into(),
                enabled: true,
                mode: ExecutionMode::Once,
                recursive: false,
                schedule: None,
                schedule_timezone: ScheduleTimezone::Local,
                rules: vec!["documents".into()],
            }],
        }
    }

    #[test]
    fn configuration_round_trips() {
        let root = tempdir().unwrap();
        let path = root.path().join("sorter.toml");
        config().save(&path).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.rules[0].id, "documents");
    }

    #[test]
    fn rejects_unknown_rule_reference() {
        let mut value = config();
        value.directories[0].rules = vec!["missing".into()];
        assert!(matches!(
            value.validate(),
            Err(StorageError::UnknownRule { .. })
        ));
    }

    #[test]
    fn rejects_destination_equal_to_non_recursive_source() {
        let mut value = config();
        value.rules[0].destination = value.directories[0].path.clone();
        assert!(matches!(
            value.validate(),
            Err(StorageError::RecursiveDestinationLoop { .. })
        ));
    }

    #[test]
    fn validates_scheduled_cron_expression() {
        let mut value = config();
        value.directories[0].mode = ExecutionMode::Scheduled;
        value.directories[0].schedule = Some("0 0 2 * * *".into());
        assert!(value.validate().is_ok());

        value.directories[0].schedule = Some("not a cron expression".into());
        assert!(matches!(
            value.validate(),
            Err(StorageError::InvalidSchedule { .. })
        ));
    }

    #[test]
    fn validates_automation_limits_and_exclusions() {
        let mut value = config();
        value.settings.event_queue_capacity = 0;
        assert!(matches!(
            value.validate(),
            Err(StorageError::InvalidAutomationSetting { .. })
        ));

        value.settings.event_queue_capacity = 16;
        value.settings.temporary_file_globs = vec!["[".into()];
        assert!(matches!(
            value.validate(),
            Err(StorageError::InvalidTemporaryGlob { .. })
        ));
    }

    #[test]
    fn history_records_transitions() {
        let root = tempdir().unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let id = store
            .create_operation(Path::new("/a"), Path::new("/b"), "docs")
            .unwrap();
        store
            .update_operation(id, OperationState::Completed, None)
            .unwrap();
        let entries = store.recent(10).unwrap();
        assert_eq!(entries[0].state, "COMPLETED");
    }

    #[test]
    fn journal_persists_transfer_metadata() {
        let root = tempdir().unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let id = store
            .create_operation(Path::new("/a"), Path::new("/b"), "docs")
            .unwrap();
        let fingerprint = FileFingerprint {
            size: 42,
            modified_unix_ns: Some(123),
            device: Some(4),
            file_id: Some(5),
        };
        store
            .record_prepared(
                id,
                Path::new("/actual-b"),
                Some(Path::new("/.partial")),
                TransferKind::StagedCopy,
                &fingerprint,
            )
            .unwrap();
        store
            .update_operation(id, OperationState::Verified, None)
            .unwrap();

        let operation = store.operation(id).unwrap();
        assert_eq!(operation.destination, Path::new("/actual-b"));
        assert_eq!(
            operation.temporary_path.as_deref(),
            Some(Path::new("/.partial"))
        );
        assert_eq!(operation.transfer_kind, Some(TransferKind::StagedCopy));
        assert_eq!(operation.source_fingerprint, Some(fingerprint));
        assert_eq!(operation.state, OperationState::Verified);
    }

    #[test]
    fn unfinished_operation_locks_source() {
        let root = tempdir().unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let source = Path::new("/same-source");
        let id = store
            .create_operation(source, Path::new("/first"), "docs")
            .unwrap();
        assert!(matches!(
            store.create_operation(source, Path::new("/second"), "docs"),
            Err(StorageError::SourceAlreadyActive(_))
        ));
        store
            .update_operation(id, OperationState::Completed, None)
            .unwrap();
        assert!(store
            .create_operation(source, Path::new("/second"), "docs")
            .is_ok());
    }

    #[test]
    fn phase_two_database_is_migrated_and_locked() {
        let root = tempdir().unwrap();
        let database = root.path().join("history.db");
        {
            let connection = Connection::open(&database).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE operations (
                        id INTEGER PRIMARY KEY,
                        source TEXT NOT NULL,
                        destination TEXT NOT NULL,
                        rule_id TEXT NOT NULL,
                        state TEXT NOT NULL,
                        error TEXT,
                        created_at_unix_ms INTEGER NOT NULL,
                        updated_at_unix_ms INTEGER NOT NULL
                    );
                    INSERT INTO operations
                        (source, destination, rule_id, state,
                         created_at_unix_ms, updated_at_unix_ms)
                    VALUES ('/legacy-source', '/legacy-destination', 'docs',
                            'IN_PROGRESS', 1, 1);",
                )
                .unwrap();
        }

        let store = HistoryStore::open(&database).unwrap();
        let operation = store.operation(1).unwrap();
        assert_eq!(operation.source, Path::new("/legacy-source"));
        assert!(matches!(
            store.create_operation(
                Path::new("/legacy-source"),
                Path::new("/another-destination"),
                "docs"
            ),
            Err(StorageError::SourceAlreadyActive(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn journal_round_trips_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;

        let root = tempdir().unwrap();
        let store = HistoryStore::open(&root.path().join("history.db")).unwrap();
        let source = PathBuf::from(std::ffi::OsString::from_vec(b"/in/\xff".to_vec()));
        let destination = PathBuf::from(std::ffi::OsString::from_vec(b"/out/\xfe".to_vec()));
        let id = store
            .create_operation(&source, &destination, "docs")
            .unwrap();
        let operation = store.operation(id).unwrap();
        assert_eq!(operation.source, source);
        assert_eq!(operation.destination, destination);
    }
}
