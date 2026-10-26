//! TOML configuration and `SQLite` operation history.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sorter_core::{MatchEngine, Rule};
use sorter_fs::CollisionPolicy;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
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
    #[error("destination for rule {rule_id} is inside recursive source directory {directory_id}: {destination}")]
    RecursiveDestinationLoop {
        directory_id: String,
        rule_id: String,
        destination: PathBuf,
    },
    #[error(transparent)]
    Core(#[from] sorter_core::CoreError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            collision_policy: CollisionPolicy::Skip,
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
                if directory.recursive && rule.destination.starts_with(&directory.path) {
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OperationState {
    Planned,
    InProgress,
    Copied,
    Verified,
    Completed,
    Failed,
    RecoveryRequired,
    Skipped,
}

impl OperationState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "PLANNED",
            Self::InProgress => "IN_PROGRESS",
            Self::Copied => "COPIED",
            Self::Verified => "VERIFIED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::Skipped => "SKIPPED",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HistoryEntry {
    pub id: i64,
    pub source: String,
    pub destination: String,
    pub rule_id: String,
    pub state: String,
    pub error: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
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
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS operations (
                id INTEGER PRIMARY KEY,
                source TEXT NOT NULL,
                destination TEXT NOT NULL,
                rule_id TEXT NOT NULL,
                state TEXT NOT NULL,
                error TEXT,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS operations_updated_idx
                ON operations(updated_at_unix_ms DESC);",
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
        self.connection.execute(
            "INSERT INTO operations
             (source, destination, rule_id, state, created_at_unix_ms, updated_at_unix_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![
                source.to_string_lossy(),
                destination.to_string_lossy(),
                rule_id,
                OperationState::Planned.as_str(),
                now
            ],
        )?;
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
            "UPDATE operations SET destination = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
            params![destination.to_string_lossy(), timestamp_ms()?, id],
        )?;
        Ok(())
    }

    pub fn recent(&self, limit: u32) -> Result<Vec<HistoryEntry>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT id, source, destination, rule_id, state, error,
                    created_at_unix_ms, updated_at_unix_ms
             FROM operations ORDER BY updated_at_unix_ms DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                source: row.get(1)?,
                destination: row.get(2)?,
                rule_id: row.get(3)?,
                state: row.get(4)?,
                error: row.get(5)?,
                created_at_unix_ms: row.get(6)?,
                updated_at_unix_ms: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
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
}
