use serde::Serialize;
use sorter_core::{MatchEngine, PlannedMove};
use sorter_runtime::{
    run_once, run_scheduled, run_watch, AutomationReport, AutomationSelection, OneShotOptions,
    RunReport, ShutdownToken,
};
use sorter_storage::{
    Config, DirectoryConfig, ExecutionMode, HistoryEntry, HistoryStore, Settings,
    CURRENT_CONFIG_VERSION,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize)]
pub struct DesktopSnapshot {
    pub config: Config,
    pub history: Vec<HistoryEntry>,
    pub config_path: PathBuf,
    pub database_path: PathBuf,
    pub automation: Vec<AutomationStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationStatus {
    pub directory_id: String,
    pub mode: String,
    pub state: AutomationStateKind,
    pub started_at_unix_ms: i64,
    pub executions: u64,
    pub item_failures: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationStateKind {
    Running,
    Stopping,
    Completed,
    Failed,
}

struct RunningAutomation {
    mode: String,
    shutdown: ShutdownToken,
    handle: JoinHandle<Result<AutomationReport, String>>,
    started_at_unix_ms: i64,
    stopping: bool,
}

#[derive(Default)]
struct AutomationManager {
    running: HashMap<String, RunningAutomation>,
    completed: HashMap<String, AutomationStatus>,
}

pub struct DesktopBackend {
    config_path: PathBuf,
    database_path: PathBuf,
    operation_lock: Mutex<()>,
    automation: Mutex<AutomationManager>,
}

impl DesktopBackend {
    pub fn new(config_path: PathBuf, database_path: PathBuf) -> Result<Self, String> {
        if !config_path.exists() {
            empty_config().save(&config_path).map_err(stringify)?;
        }
        Config::load(&config_path).map_err(stringify)?;
        Ok(Self {
            config_path,
            database_path,
            operation_lock: Mutex::new(()),
            automation: Mutex::new(AutomationManager::default()),
        })
    }

    pub fn snapshot(&self) -> Result<DesktopSnapshot, String> {
        let config = self.load_config()?;
        let history = HistoryStore::open(&self.database_path)
            .map_err(stringify)?
            .recent(100)
            .map_err(stringify)?;
        Ok(DesktopSnapshot {
            config,
            history,
            config_path: self.config_path.clone(),
            database_path: self.database_path.clone(),
            automation: self.automation_status()?,
        })
    }

    pub fn save_configuration(&self, config: &Config) -> Result<Config, String> {
        config.validate().map_err(stringify)?;
        let _operation = self.lock_operations()?;
        let mut automation = self.lock_automation()?;
        cleanup_finished(&mut automation);
        if !automation.running.is_empty() {
            return Err("stop active automation before saving configuration".into());
        }
        config.save(&self.config_path).map_err(stringify)?;
        Ok(config.clone())
    }

    pub fn test_filename(config: &Config, filename: &str) -> Result<Option<PlannedMove>, String> {
        if filename.trim().is_empty() {
            return Err("filename must not be empty".into());
        }
        MatchEngine::compile(&config.rules)
            .map_err(stringify)?
            .plan(Path::new(filename))
            .map_err(stringify)
    }

    pub fn preview_directory(&self, directory_id: &str) -> Result<RunReport, String> {
        let _operation = self.lock_operations()?;
        let config = self.load_config()?;
        let directory = find_directory(&config, directory_id)?;
        let rules = config.rules_for(directory);
        run_once(
            &OneShotOptions {
                source: &directory.path,
                recursive: directory.recursive,
                dry_run: true,
                collision_policy: config.settings.collision_policy,
                rules: &rules,
            },
            None,
        )
        .map_err(stringify)
    }

    pub fn execute_directory(&self, directory_id: &str) -> Result<RunReport, String> {
        let _operation = self.lock_operations()?;
        {
            let mut automation = self.lock_automation()?;
            cleanup_finished(&mut automation);
            if automation.running.contains_key(directory_id) {
                return Err(format!(
                    "directory {directory_id} is already controlled by automation"
                ));
            }
        }
        let config = self.load_config()?;
        let directory = find_directory(&config, directory_id)?;
        let rules = config.rules_for(directory);
        let history = HistoryStore::open(&self.database_path).map_err(stringify)?;
        run_once(
            &OneShotOptions {
                source: &directory.path,
                recursive: directory.recursive,
                dry_run: false,
                collision_policy: config.settings.collision_policy,
                rules: &rules,
            },
            Some(&history),
        )
        .map_err(stringify)
    }

    pub fn history(&self, limit: u32) -> Result<Vec<HistoryEntry>, String> {
        HistoryStore::open(&self.database_path)
            .map_err(stringify)?
            .recent(limit.clamp(1, 1_000))
            .map_err(stringify)
    }

    pub fn start_automation(&self, directory_id: &str) -> Result<Vec<AutomationStatus>, String> {
        let _operation = self.lock_operations()?;
        let config = self.load_config()?;
        let directory = find_directory(&config, directory_id)?;
        if !directory.enabled {
            return Err(format!("directory {directory_id} is disabled"));
        }
        if directory.mode == ExecutionMode::Once {
            return Err(format!(
                "directory {directory_id} uses one-shot mode; run it from Preview"
            ));
        }

        let mut automation = self.lock_automation()?;
        cleanup_finished(&mut automation);
        if automation.running.contains_key(directory_id) {
            return Err(format!("automation is already running for {directory_id}"));
        }
        automation.completed.remove(directory_id);

        let id = directory_id.to_owned();
        let thread_id = id.clone();
        let mode = directory.mode;
        let mode_name = mode_name(mode).to_owned();
        let database_path = self.database_path.clone();
        let shutdown = ShutdownToken::default();
        let thread_shutdown = shutdown.clone();
        let selection = AutomationSelection {
            directory_id: Some(id.clone()),
        };
        let handle = thread::Builder::new()
            .name(format!("sorter-desktop-{mode_name}-{id}"))
            .spawn(move || {
                let result = match mode {
                    ExecutionMode::Watch => {
                        run_watch(&config, &database_path, &selection, &thread_shutdown)
                    }
                    ExecutionMode::Scheduled => {
                        run_scheduled(&config, &database_path, &selection, &thread_shutdown)
                    }
                    ExecutionMode::Once => unreachable!("one-shot mode was rejected"),
                };
                result.map_err(|error| format!("{thread_id}: {error}"))
            })
            .map_err(|error| format!("could not start automation thread: {error}"))?;

        automation.running.insert(
            id,
            RunningAutomation {
                mode: mode_name,
                shutdown,
                handle,
                started_at_unix_ms: timestamp_ms(),
                stopping: false,
            },
        );
        Ok(statuses(&automation))
    }

    pub fn stop_automation(&self, directory_id: &str) -> Result<Vec<AutomationStatus>, String> {
        let mut automation = self.lock_automation()?;
        cleanup_finished(&mut automation);
        let worker = automation
            .running
            .get_mut(directory_id)
            .ok_or_else(|| format!("automation is not running for {directory_id}"))?;
        worker.stopping = true;
        worker.shutdown.request();
        Ok(statuses(&automation))
    }

    pub fn automation_status(&self) -> Result<Vec<AutomationStatus>, String> {
        let mut automation = self.lock_automation()?;
        cleanup_finished(&mut automation);
        Ok(statuses(&automation))
    }

    fn load_config(&self) -> Result<Config, String> {
        Config::load(&self.config_path).map_err(stringify)
    }

    fn lock_operations(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.operation_lock
            .lock()
            .map_err(|_| "desktop operation lock is poisoned".into())
    }

    fn lock_automation(&self) -> Result<std::sync::MutexGuard<'_, AutomationManager>, String> {
        self.automation
            .lock()
            .map_err(|_| "desktop automation lock is poisoned".into())
    }
}

impl Drop for DesktopBackend {
    fn drop(&mut self) {
        if let Ok(manager) = self.automation.get_mut() {
            for worker in manager.running.values() {
                worker.shutdown.request();
            }
            for (_, worker) in manager.running.drain() {
                let _ = worker.handle.join();
            }
        }
    }
}

fn find_directory<'a>(
    config: &'a Config,
    directory_id: &str,
) -> Result<&'a DirectoryConfig, String> {
    config
        .directories
        .iter()
        .find(|directory| directory.id == directory_id)
        .ok_or_else(|| format!("unknown directory id: {directory_id}"))
}

fn empty_config() -> Config {
    Config {
        version: CURRENT_CONFIG_VERSION,
        settings: Settings::default(),
        rules: Vec::new(),
        directories: Vec::new(),
    }
}

fn cleanup_finished(manager: &mut AutomationManager) {
    let finished: Vec<_> = manager
        .running
        .iter()
        .filter(|(_, worker)| worker.handle.is_finished())
        .map(|(id, _)| id.clone())
        .collect();
    for id in finished {
        let Some(worker) = manager.running.remove(&id) else {
            continue;
        };
        let status = completion_status(&id, worker);
        manager.completed.insert(id, status);
    }
}

fn completion_status(directory_id: &str, worker: RunningAutomation) -> AutomationStatus {
    let mut executions = 0;
    let mut item_failures = 0;
    let mut error = None;
    match worker.handle.join() {
        Ok(Ok(report)) => {
            executions = report.workers.iter().map(|entry| entry.executions).sum();
            item_failures = report.workers.iter().map(|entry| entry.item_failures).sum();
            let failures: Vec<_> = report
                .workers
                .into_iter()
                .filter_map(|entry| entry.fatal_error)
                .collect();
            if !failures.is_empty() {
                error = Some(failures.join("; "));
            }
        }
        Ok(Err(failure)) => error = Some(failure),
        Err(_) => error = Some("automation thread panicked".into()),
    }
    AutomationStatus {
        directory_id: directory_id.to_owned(),
        mode: worker.mode,
        state: if error.is_some() {
            AutomationStateKind::Failed
        } else {
            AutomationStateKind::Completed
        },
        started_at_unix_ms: worker.started_at_unix_ms,
        executions,
        item_failures,
        error,
    }
}

fn statuses(manager: &AutomationManager) -> Vec<AutomationStatus> {
    let mut result: Vec<_> = manager.completed.values().cloned().collect();
    result.extend(
        manager
            .running
            .iter()
            .map(|(directory_id, worker)| AutomationStatus {
                directory_id: directory_id.clone(),
                mode: worker.mode.clone(),
                state: if worker.stopping {
                    AutomationStateKind::Stopping
                } else {
                    AutomationStateKind::Running
                },
                started_at_unix_ms: worker.started_at_unix_ms,
                executions: 0,
                item_failures: 0,
                error: None,
            }),
    );
    result.sort_by(|left, right| left.directory_id.cmp(&right.directory_id));
    result
}

const fn mode_name(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Once => "once",
        ExecutionMode::Watch => "watch",
        ExecutionMode::Scheduled => "scheduled",
    }
}

fn timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn stringify(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorter_core::{MatchOperator, MatchSpec, Rule};
    use sorter_fs::CollisionPolicy;
    use sorter_storage::ScheduleTimezone;
    use std::fs;
    use tempfile::tempdir;

    fn configured_backend(root: &Path) -> DesktopBackend {
        let backend = DesktopBackend::new(root.join("sorter.toml"), root.join("sorter.db"))
            .expect("backend should initialize");
        let source = root.join("incoming");
        let destination = root.join("sorted");
        fs::create_dir(&source).unwrap();
        backend
            .save_configuration(&Config {
                version: CURRENT_CONFIG_VERSION,
                settings: Settings {
                    collision_policy: CollisionPolicy::Error,
                    ..Settings::default()
                },
                rules: vec![Rule {
                    id: "documents".into(),
                    enabled: true,
                    priority: 10,
                    destination,
                    match_spec: MatchSpec {
                        operator: MatchOperator::All,
                        extensions: vec!["pdf".into()],
                        ..MatchSpec::default()
                    },
                }],
                directories: vec![DirectoryConfig {
                    id: "downloads".into(),
                    path: source,
                    enabled: true,
                    mode: ExecutionMode::Once,
                    recursive: false,
                    schedule: None,
                    schedule_timezone: ScheduleTimezone::Local,
                    rules: vec!["documents".into()],
                }],
            })
            .unwrap();
        backend
    }

    #[test]
    fn initializes_an_empty_valid_configuration() {
        let root = tempdir().unwrap();
        let backend = DesktopBackend::new(
            root.path().join("config/sorter.toml"),
            root.path().join("data/sorter.db"),
        )
        .unwrap();
        let config = backend.load_config().unwrap();
        assert!(config.rules.is_empty());
        assert!(config.directories.is_empty());
    }

    #[test]
    fn tests_rules_and_previews_without_mutation_or_history() {
        let root = tempdir().unwrap();
        let backend = configured_backend(root.path());
        let source = root.path().join("incoming/report.pdf");
        fs::write(&source, b"report").unwrap();
        let config = backend.load_config().unwrap();

        let matched = DesktopBackend::test_filename(&config, "REPORT.PDF")
            .unwrap()
            .unwrap();
        assert_eq!(matched.rule_id, "documents");

        let report = backend.preview_directory("downloads").unwrap();
        assert_eq!(report.matched, 1);
        assert!(source.exists());
        assert!(!root.path().join("sorter.db").exists());
    }

    #[test]
    fn execution_uses_runtime_and_records_history() {
        let root = tempdir().unwrap();
        let backend = configured_backend(root.path());
        let source = root.path().join("incoming/report.pdf");
        fs::write(&source, b"report").unwrap();

        let report = backend.execute_directory("downloads").unwrap();
        assert_eq!(report.moved, 1);
        assert!(!source.exists());
        assert!(root.path().join("sorted/report.pdf").exists());
        let history = backend.history(10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].state, "COMPLETED");
    }
}
