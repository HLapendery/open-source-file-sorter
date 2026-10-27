use super::{run_candidate, run_once_filtered, validate_source, OneShotOptions, RuntimeError};
use chrono::{Local, Utc};
use cron::Schedule;
use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{
    Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
};
use serde::Serialize;
use sorter_core::{MatchEngine, Rule};
use sorter_storage::{
    Config, DirectoryConfig, ExecutionMode, HistoryStore, ScheduleTimezone, Settings,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use tracing::{error, info, warn};
use walkdir::{IntoIter as WalkIter, WalkDir};

#[derive(Debug, Clone, Default)]
pub struct AutomationSelection {
    pub directory_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutomationReport {
    pub mode: &'static str,
    pub directories_started: u64,
    pub worker_failures: u64,
    pub workers: Vec<WorkerReport>,
}

impl AutomationReport {
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.worker_failures > 0
    }
}

#[derive(Debug, Serialize)]
pub struct WorkerReport {
    pub directory_id: String,
    pub executions: u64,
    pub item_failures: u64,
    pub fatal_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct ShutdownToken {
    inner: Arc<(Mutex<bool>, Condvar)>,
}

impl ShutdownToken {
    pub fn request(&self) {
        let (lock, condition) = &*self.inner;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
        condition.notify_all();
    }

    #[must_use]
    pub fn is_requested(&self) -> bool {
        let (lock, _) = &*self.inner;
        *lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait(&self, duration: Duration) -> bool {
        let (lock, condition) = &*self.inner;
        let stopped = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *stopped {
            return true;
        }
        let (stopped, _) = condition
            .wait_timeout(stopped, duration)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *stopped
    }
}

#[derive(Clone)]
struct WorkerConfig {
    id: String,
    path: PathBuf,
    recursive: bool,
    schedule: Option<String>,
    schedule_timezone: ScheduleTimezone,
    rules: Vec<Rule>,
}

pub fn run_watch(
    config: &Config,
    database_path: &Path,
    selection: &AutomationSelection,
    shutdown: &ShutdownToken,
) -> Result<AutomationReport, RuntimeError> {
    let workers = select_directories(config, ExecutionMode::Watch, selection)?;
    drop(HistoryStore::open(database_path)?);
    let gate = Arc::new(ExecutionGate::new(
        config.settings.max_concurrent_operations,
    ));
    let settings = config.settings.clone();
    Ok(run_worker_threads(
        "watch",
        workers,
        database_path,
        shutdown,
        move |worker, db, stop| watch_worker(&worker, &settings, &db, &stop, &gate),
    ))
}

pub fn run_scheduled(
    config: &Config,
    database_path: &Path,
    selection: &AutomationSelection,
    shutdown: &ShutdownToken,
) -> Result<AutomationReport, RuntimeError> {
    let workers = select_directories(config, ExecutionMode::Scheduled, selection)?;
    drop(HistoryStore::open(database_path)?);
    let gate = Arc::new(ExecutionGate::new(
        config.settings.max_concurrent_operations,
    ));
    let settings = config.settings.clone();
    Ok(run_worker_threads(
        "schedule",
        workers,
        database_path,
        shutdown,
        move |worker, db, stop| scheduled_worker(&worker, &settings, &db, &stop, &gate),
    ))
}

fn select_directories(
    config: &Config,
    mode: ExecutionMode,
    selection: &AutomationSelection,
) -> Result<Vec<WorkerConfig>, RuntimeError> {
    let directories: Vec<_> = if let Some(id) = selection.directory_id.as_deref() {
        let directory = config
            .directories
            .iter()
            .find(|directory| directory.id == id)
            .ok_or_else(|| RuntimeError::Automation(format!("unknown directory id: {id}")))?;
        if !directory.enabled {
            return Err(RuntimeError::Automation(format!(
                "directory {id} is disabled"
            )));
        }
        if directory.mode != mode {
            return Err(RuntimeError::Automation(format!(
                "directory {id} is configured for {:?}, not {:?}",
                directory.mode, mode
            )));
        }
        vec![directory]
    } else {
        config
            .directories
            .iter()
            .filter(|directory| directory.enabled && directory.mode == mode)
            .collect()
    };
    if directories.is_empty() {
        return Err(RuntimeError::Automation(format!(
            "no enabled {mode:?} directories are configured"
        )));
    }
    Ok(directories
        .into_iter()
        .map(|directory| worker_config(config, directory))
        .collect())
}

fn worker_config(config: &Config, directory: &DirectoryConfig) -> WorkerConfig {
    WorkerConfig {
        id: directory.id.clone(),
        path: directory.path.clone(),
        recursive: directory.recursive,
        schedule: directory.schedule.clone(),
        schedule_timezone: directory.schedule_timezone,
        rules: config.rules_for(directory),
    }
}

fn run_worker_threads<F>(
    mode: &'static str,
    workers: Vec<WorkerConfig>,
    database_path: &Path,
    shutdown: &ShutdownToken,
    worker_fn: F,
) -> AutomationReport
where
    F: Fn(WorkerConfig, PathBuf, ShutdownToken) -> Result<WorkerReport, RuntimeError>
        + Send
        + Sync
        + 'static,
{
    let worker_fn = Arc::new(worker_fn);
    let mut handles = Vec::with_capacity(workers.len());
    let mut immediate_failures = Vec::new();
    for worker in workers {
        let id = worker.id.clone();
        let thread_id = id.clone();
        let database = database_path.to_path_buf();
        let stop = shutdown.clone();
        let function = Arc::clone(&worker_fn);
        match thread::Builder::new()
            .name(format!("sorter-{mode}-{id}"))
            .spawn(move || {
                let result = function(worker, database, stop);
                if let Err(failure) = &result {
                    error!(directory = %thread_id, error = %failure, "automation worker stopped");
                }
                result
            }) {
            Ok(handle) => handles.push((id, handle)),
            Err(failure) => immediate_failures.push(WorkerReport {
                directory_id: id,
                executions: 0,
                item_failures: 0,
                fatal_error: Some(format!("could not start worker thread: {failure}")),
            }),
        }
    }

    let directories_started = u64::try_from(handles.len()).unwrap_or(u64::MAX);
    let mut reports = immediate_failures;
    for (directory_id, handle) in handles {
        match handle.join() {
            Ok(Ok(report)) => reports.push(report),
            Ok(Err(failure)) => reports.push(WorkerReport {
                directory_id: directory_id.clone(),
                executions: 0,
                item_failures: 0,
                fatal_error: Some(failure.to_string()),
            }),
            Err(_) => reports.push(WorkerReport {
                directory_id,
                executions: 0,
                item_failures: 0,
                fatal_error: Some("worker thread panicked".into()),
            }),
        }
    }
    let worker_failures = reports
        .iter()
        .filter(|report| report.fatal_error.is_some())
        .count()
        .try_into()
        .unwrap_or(u64::MAX);
    AutomationReport {
        mode,
        directories_started,
        worker_failures,
        workers: reports,
    }
}

#[allow(clippy::too_many_lines)]
fn watch_worker(
    worker: &WorkerConfig,
    settings: &Settings,
    database_path: &Path,
    shutdown: &ShutdownToken,
    gate: &ExecutionGate,
) -> Result<WorkerReport, RuntimeError> {
    validate_source(&worker.path)?;
    let engine = MatchEngine::compile(&worker.rules)?;
    let exclusions = compile_exclusions(&settings.temporary_file_globs)?;
    let history = HistoryStore::open(database_path)?;
    let queue_capacity = settings.event_queue_capacity;
    let (sender, receiver) = mpsc::sync_channel(queue_capacity);
    let overflowed = Arc::new(AtomicBool::new(false));
    let callback_overflow = Arc::clone(&overflowed);
    let mut watcher = RecommendedWatcher::new(
        move |event| send_event(&sender, &callback_overflow, event),
        NotifyConfig::default().with_follow_symlinks(false),
    )
    .map_err(|failure| automation_error("create native watcher", failure))?;
    watcher
        .watch(
            &worker.path,
            if worker.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            },
        )
        .map_err(|failure| automation_error("watch source directory", failure))?;

    info!(directory = %worker.id, path = %worker.path.display(), "watch worker started");
    let debounce = Duration::from_millis(settings.watch_debounce_ms);
    let stabilization = Duration::from_millis(settings.stabilization_delay_ms);
    let reconciliation = Duration::from_secs(settings.reconciliation_interval_seconds);
    let tick = debounce
        .min(stabilization)
        .clamp(Duration::from_millis(50), Duration::from_secs(1));
    let destinations: Vec<_> = worker
        .rules
        .iter()
        .map(|rule| rule.destination.clone())
        .collect();
    let mut pending = PendingTracker::new(queue_capacity);
    let mut scanner = ReconcileState::new();
    let mut executions = 0_u64;
    let mut item_failures = 0_u64;

    while !shutdown.is_requested() {
        match receiver.recv_timeout(tick) {
            Ok(event) => handle_event(
                event,
                worker,
                &exclusions,
                &destinations,
                &mut pending,
                &overflowed,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(RuntimeError::Automation(format!(
                    "watch event channel disconnected for {}",
                    worker.id
                )));
            }
        }
        drain_events(
            &receiver,
            worker,
            &exclusions,
            &destinations,
            &mut pending,
            &overflowed,
        );
        if overflowed.swap(false, Ordering::AcqRel) {
            warn!(directory = %worker.id, "watch event queue overflowed; reconciliation requested");
            scanner.request();
        }
        scanner.maybe_start(worker, reconciliation);
        scanner.fill(
            worker,
            &exclusions,
            &destinations,
            &mut pending,
            &overflowed,
        );

        for path in pending.ready_paths(Instant::now(), debounce, stabilization) {
            let Some(_permit) = gate.acquire(shutdown) else {
                break;
            };
            match run_candidate(&engine, &path, settings.collision_policy, Some(&history)) {
                Ok(Some(item)) => {
                    executions += 1;
                    if item.error.is_some() {
                        item_failures += 1;
                        warn!(directory = %worker.id, source = %item.source.display(), "watch item failed");
                    }
                }
                Ok(None) => {}
                Err(failure) => {
                    item_failures += 1;
                    error!(directory = %worker.id, source = %path.display(), error = %failure, "watch item failed");
                }
            }
        }
    }
    info!(directory = %worker.id, "watch worker stopped");
    Ok(WorkerReport {
        directory_id: worker.id.clone(),
        executions,
        item_failures,
        fatal_error: None,
    })
}

fn send_event(
    sender: &SyncSender<notify::Result<Event>>,
    overflowed: &AtomicBool,
    event: notify::Result<Event>,
) {
    match sender.try_send(event) {
        Ok(()) | Err(TrySendError::Disconnected(_)) => {}
        Err(TrySendError::Full(_)) => {
            overflowed.store(true, Ordering::Release);
        }
    }
}

fn drain_events(
    receiver: &Receiver<notify::Result<Event>>,
    worker: &WorkerConfig,
    exclusions: &GlobSet,
    destinations: &[PathBuf],
    pending: &mut PendingTracker,
    overflowed: &AtomicBool,
) {
    while let Ok(event) = receiver.try_recv() {
        handle_event(event, worker, exclusions, destinations, pending, overflowed);
    }
}

fn handle_event(
    event: notify::Result<Event>,
    worker: &WorkerConfig,
    exclusions: &GlobSet,
    destinations: &[PathBuf],
    pending: &mut PendingTracker,
    overflowed: &AtomicBool,
) {
    match event {
        Ok(event) if matches!(event.kind, EventKind::Access(_)) => {}
        Ok(event) => {
            for path in event.paths {
                if is_candidate_path(&path, worker, exclusions, destinations)
                    && !pending.touch(path, Instant::now())
                {
                    overflowed.store(true, Ordering::Release);
                }
            }
        }
        Err(failure) => {
            warn!(directory = %worker.id, error = %failure, "native watcher reported an error");
            overflowed.store(true, Ordering::Release);
        }
    }
}

fn is_candidate_path(
    path: &Path,
    worker: &WorkerConfig,
    exclusions: &GlobSet,
    destinations: &[PathBuf],
) -> bool {
    if destinations
        .iter()
        .any(|destination| path.starts_with(destination))
    {
        return false;
    }
    if exclusions.is_match(path.file_name().unwrap_or_default()) {
        return false;
    }
    if worker.recursive {
        path.starts_with(&worker.path)
    } else {
        path.parent() == Some(worker.path.as_path())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    length: u64,
    modified: Option<SystemTime>,
}

struct PendingFile {
    last_event: Instant,
    snapshot: Option<FileSnapshot>,
    stable_since: Option<Instant>,
}

struct PendingTracker {
    files: HashMap<PathBuf, PendingFile>,
    capacity: usize,
}

impl PendingTracker {
    fn new(capacity: usize) -> Self {
        Self {
            files: HashMap::with_capacity(capacity.min(4_096)),
            capacity,
        }
    }

    fn touch(&mut self, path: PathBuf, now: Instant) -> bool {
        if let Some(file) = self.files.get_mut(&path) {
            file.last_event = now;
            file.snapshot = None;
            file.stable_since = None;
            return true;
        }
        if self.files.len() >= self.capacity {
            return false;
        }
        self.files.insert(
            path,
            PendingFile {
                last_event: now,
                snapshot: None,
                stable_since: None,
            },
        );
        true
    }

    fn ready_paths(
        &mut self,
        now: Instant,
        debounce: Duration,
        stabilization: Duration,
    ) -> Vec<PathBuf> {
        let mut ready = Vec::new();
        let mut remove = Vec::new();
        for (path, pending) in &mut self.files {
            if now.saturating_duration_since(pending.last_event) < debounce {
                continue;
            }
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata
                }
                Ok(_) => {
                    remove.push(path.clone());
                    continue;
                }
                Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {
                    remove.push(path.clone());
                    continue;
                }
                Err(failure) => {
                    warn!(path = %path.display(), error = %failure, "could not inspect pending file");
                    continue;
                }
            };
            let snapshot = FileSnapshot {
                length: metadata.len(),
                modified: metadata.modified().ok(),
            };
            if pending.snapshot.as_ref() != Some(&snapshot) {
                pending.snapshot = Some(snapshot);
                pending.stable_since = Some(now);
                continue;
            }
            if pending
                .stable_since
                .is_some_and(|since| now.saturating_duration_since(since) >= stabilization)
            {
                ready.push(path.clone());
                remove.push(path.clone());
            }
        }
        for path in remove {
            self.files.remove(&path);
        }
        ready
    }
}

struct ReconcileState {
    iterator: Option<WalkIter>,
    next_scan: Instant,
    requested: bool,
}

impl ReconcileState {
    fn new() -> Self {
        Self {
            iterator: None,
            next_scan: Instant::now(),
            requested: true,
        }
    }

    fn request(&mut self) {
        self.requested = true;
    }

    fn maybe_start(&mut self, worker: &WorkerConfig, interval: Duration) {
        if self.iterator.is_none() && (self.requested || Instant::now() >= self.next_scan) {
            let max_depth = if worker.recursive { usize::MAX } else { 1 };
            self.iterator = Some(
                WalkDir::new(&worker.path)
                    .follow_links(false)
                    .min_depth(1)
                    .max_depth(max_depth)
                    .into_iter(),
            );
            self.requested = false;
            self.next_scan = Instant::now() + interval;
            info!(directory = %worker.id, "reconciliation scan started");
        }
    }

    fn fill(
        &mut self,
        worker: &WorkerConfig,
        exclusions: &GlobSet,
        destinations: &[PathBuf],
        pending: &mut PendingTracker,
        overflowed: &AtomicBool,
    ) {
        let Some(iterator) = &mut self.iterator else {
            return;
        };
        while pending.files.len() < pending.capacity {
            match iterator.next() {
                Some(Ok(entry)) if entry.file_type().is_file() => {
                    let path = entry.into_path();
                    if is_candidate_path(&path, worker, exclusions, destinations)
                        && !pending.touch(path, Instant::now())
                    {
                        overflowed.store(true, Ordering::Release);
                        break;
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(failure)) => {
                    warn!(directory = %worker.id, error = %failure, "reconciliation entry failed");
                }
                None => {
                    self.iterator = None;
                    info!(directory = %worker.id, "reconciliation scan completed");
                    break;
                }
            }
        }
    }
}

fn scheduled_worker(
    worker: &WorkerConfig,
    settings: &Settings,
    database_path: &Path,
    shutdown: &ShutdownToken,
    gate: &ExecutionGate,
) -> Result<WorkerReport, RuntimeError> {
    validate_source(&worker.path)?;
    let expression = worker.schedule.as_deref().ok_or_else(|| {
        RuntimeError::Automation(format!("scheduled directory {} has no cron", worker.id))
    })?;
    let schedule = Schedule::from_str(expression)
        .map_err(|failure| automation_error("parse cron schedule", failure))?;
    let exclusions = compile_exclusions(&settings.temporary_file_globs)?;
    let history = HistoryStore::open(database_path)?;
    let mut executions = 0_u64;
    let mut item_failures = 0_u64;
    info!(directory = %worker.id, schedule = expression, timezone = ?worker.schedule_timezone, "schedule worker started");

    while !shutdown.is_requested() {
        let Some(next) = next_occurrence(&schedule, worker.schedule_timezone) else {
            return Err(RuntimeError::Automation(format!(
                "schedule for {} has no future occurrence",
                worker.id
            )));
        };
        info!(directory = %worker.id, next_unix_ms = system_time_millis(next), "next scheduled run");
        if wait_until(shutdown, next) {
            break;
        }
        let Some(_permit) = gate.acquire(shutdown) else {
            break;
        };
        let report = run_once_filtered(
            &OneShotOptions {
                source: &worker.path,
                recursive: worker.recursive,
                dry_run: false,
                collision_policy: settings.collision_policy,
                rules: &worker.rules,
            },
            Some(&history),
            Some(&exclusions),
        );
        executions += 1;
        match report {
            Ok(report) => {
                item_failures = item_failures
                    .saturating_add(report.failed)
                    .saturating_add(u64::try_from(report.scan_errors.len()).unwrap_or(u64::MAX));
                info!(
                    directory = %worker.id,
                    scanned = report.scanned,
                    moved = report.moved,
                    failed = report.failed,
                    "scheduled run completed"
                );
            }
            Err(failure) => {
                item_failures += 1;
                error!(directory = %worker.id, error = %failure, "scheduled run failed");
            }
        }
    }
    info!(directory = %worker.id, "schedule worker stopped");
    Ok(WorkerReport {
        directory_id: worker.id.clone(),
        executions,
        item_failures,
        fatal_error: None,
    })
}

fn next_occurrence(schedule: &Schedule, timezone: ScheduleTimezone) -> Option<SystemTime> {
    match timezone {
        ScheduleTimezone::Local => schedule
            .upcoming(Local)
            .next()
            .map(|time| time.with_timezone(&Utc).into()),
        ScheduleTimezone::Utc => schedule.upcoming(Utc).next().map(Into::into),
    }
}

fn wait_until(shutdown: &ShutdownToken, deadline: SystemTime) -> bool {
    loop {
        if shutdown.is_requested() {
            return true;
        }
        let Ok(remaining) = deadline.duration_since(SystemTime::now()) else {
            return false;
        };
        if remaining.is_zero() {
            return false;
        }
        if shutdown.wait(remaining.min(Duration::from_secs(60))) {
            return true;
        }
    }
}

fn system_time_millis(time: SystemTime) -> u128 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

fn compile_exclusions(patterns: &[String]) -> Result<GlobSet, RuntimeError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(
            Glob::new(pattern)
                .map_err(|failure| automation_error("compile temporary-file glob", failure))?,
        );
    }
    builder
        .build()
        .map_err(|failure| automation_error("build temporary-file glob set", failure))
}

fn automation_error(context: &str, failure: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Automation(format!("{context}: {failure}"))
}

struct ExecutionGate {
    maximum: usize,
    active: Mutex<usize>,
    available: Condvar,
}

impl ExecutionGate {
    const fn new(maximum: usize) -> Self {
        Self {
            maximum,
            active: Mutex::new(0),
            available: Condvar::new(),
        }
    }

    fn acquire<'a>(&'a self, shutdown: &ShutdownToken) -> Option<ExecutionPermit<'a>> {
        let mut active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active >= self.maximum {
            if shutdown.is_requested() {
                return None;
            }
            let (next, _) = self
                .available
                .wait_timeout(active, Duration::from_secs(1))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active = next;
        }
        if shutdown.is_requested() {
            return None;
        }
        *active += 1;
        Some(ExecutionPermit { gate: self })
    }
}

struct ExecutionPermit<'a> {
    gate: &'a ExecutionGate,
}

impl Drop for ExecutionPermit<'_> {
    fn drop(&mut self) {
        let mut active = self
            .gate
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active = active.saturating_sub(1);
        self.gate.available.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sorter_core::MatchSpec;
    use sorter_fs::CollisionPolicy;
    use sorter_storage::CURRENT_CONFIG_VERSION;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn pending_file_requires_two_unchanged_observations() {
        let root = tempdir().unwrap();
        let path = root.path().join("file.txt");
        fs::write(&path, b"one").unwrap();
        let start = Instant::now();
        let mut pending = PendingTracker::new(4);
        assert!(pending.touch(path.clone(), start));
        assert!(pending
            .ready_paths(
                start + Duration::from_millis(10),
                Duration::from_millis(5),
                Duration::from_millis(10)
            )
            .is_empty());
        assert!(pending
            .ready_paths(
                start + Duration::from_millis(15),
                Duration::from_millis(5),
                Duration::from_millis(10)
            )
            .is_empty());
        assert_eq!(
            pending.ready_paths(
                start + Duration::from_millis(21),
                Duration::from_millis(5),
                Duration::from_millis(10)
            ),
            [path]
        );
    }

    #[test]
    fn file_change_restarts_stabilization() {
        let root = tempdir().unwrap();
        let path = root.path().join("file.txt");
        fs::write(&path, b"one").unwrap();
        let start = Instant::now();
        let mut pending = PendingTracker::new(4);
        pending.touch(path.clone(), start);
        pending.ready_paths(
            start + Duration::from_millis(10),
            Duration::from_millis(5),
            Duration::from_millis(20),
        );
        File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"-changed")
            .unwrap();
        assert!(pending
            .ready_paths(
                start + Duration::from_millis(31),
                Duration::from_millis(5),
                Duration::from_millis(20)
            )
            .is_empty());
        assert!(pending
            .ready_paths(
                start + Duration::from_millis(50),
                Duration::from_millis(5),
                Duration::from_millis(20)
            )
            .is_empty());
        assert_eq!(
            pending.ready_paths(
                start + Duration::from_millis(52),
                Duration::from_millis(5),
                Duration::from_millis(20)
            ),
            [path]
        );
    }

    #[test]
    fn pending_queue_is_bounded() {
        let mut pending = PendingTracker::new(1);
        let now = Instant::now();
        assert!(pending.touch(PathBuf::from("first"), now));
        assert!(!pending.touch(PathBuf::from("second"), now));
    }

    #[test]
    fn event_storm_is_coalesced_and_requests_reconciliation_on_overflow() {
        let mut pending = PendingTracker::new(1);
        let path = PathBuf::from("repeated");
        let now = Instant::now();
        for offset in 0..100 {
            assert!(pending.touch(path.clone(), now + Duration::from_millis(offset)));
        }
        assert_eq!(pending.files.len(), 1);

        let (sender, _receiver) = mpsc::sync_channel(1);
        let overflowed = AtomicBool::new(false);
        send_event(&sender, &overflowed, Ok(Event::new(EventKind::Any)));
        send_event(&sender, &overflowed, Ok(Event::new(EventKind::Any)));
        assert!(overflowed.load(Ordering::Acquire));
    }

    #[test]
    fn shutdown_wakes_waiters() {
        let shutdown = ShutdownToken::default();
        let requester = shutdown.clone();
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            requester.request();
        });
        assert!(shutdown.wait(Duration::from_secs(5)));
        handle.join().unwrap();
    }

    #[test]
    fn exclusions_match_temporary_names() {
        let exclusions = compile_exclusions(&["*.part".into(), "~$*".into()]).unwrap();
        assert!(exclusions.is_match("download.part"));
        assert!(exclusions.is_match("~$report.docx"));
        assert!(!exclusions.is_match("report.pdf"));
    }

    fn text_rule(destination: PathBuf) -> Rule {
        Rule {
            id: "text".into(),
            enabled: true,
            priority: 10,
            destination,
            match_spec: MatchSpec {
                extensions: vec!["txt".into()],
                ..MatchSpec::default()
            },
        }
    }

    fn automation_config(rule: Rule, directories: Vec<DirectoryConfig>) -> Config {
        Config {
            version: CURRENT_CONFIG_VERSION,
            settings: Settings {
                collision_policy: CollisionPolicy::Error,
                watch_debounce_ms: 20,
                stabilization_delay_ms: 40,
                reconciliation_interval_seconds: 60,
                event_queue_capacity: 16,
                max_concurrent_operations: 1,
                temporary_file_globs: vec!["*.part".into()],
            },
            rules: vec![rule],
            directories,
        }
    }

    fn directory(id: &str, path: PathBuf, mode: ExecutionMode) -> DirectoryConfig {
        DirectoryConfig {
            id: id.into(),
            path,
            enabled: true,
            mode,
            recursive: false,
            schedule: (mode == ExecutionMode::Scheduled).then(|| "* * * * * *".into()),
            schedule_timezone: ScheduleTimezone::Utc,
            rules: vec!["text".into()],
        }
    }

    fn stop_after_path(path: PathBuf, shutdown: ShutdownToken) -> thread::JoinHandle<bool> {
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if path.exists() {
                    shutdown.request();
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            shutdown.request();
            false
        })
    }

    #[test]
    fn watch_reconciliation_moves_a_stable_existing_file() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("ready.txt"), b"ready").unwrap();
        let config = automation_config(
            text_rule(destination.clone()),
            vec![directory("watch", source, ExecutionMode::Watch)],
        );
        let shutdown = ShutdownToken::default();
        let waiter = stop_after_path(destination.join("ready.txt"), shutdown.clone());
        let report = run_watch(
            &config,
            &root.path().join("history.db"),
            &AutomationSelection::default(),
            &shutdown,
        )
        .unwrap();
        assert!(waiter.join().unwrap());
        assert!(!report.has_failures());
        assert_eq!(report.workers[0].executions, 1);
        assert_eq!(fs::read(destination.join("ready.txt")).unwrap(), b"ready");
    }

    #[test]
    fn watch_excludes_temporary_patterns() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("unfinished.txt"), b"partial").unwrap();
        let mut config = automation_config(
            text_rule(destination.clone()),
            vec![directory("watch", source.clone(), ExecutionMode::Watch)],
        );
        config.settings.temporary_file_globs = vec!["*.txt".into()];
        let shutdown = ShutdownToken::default();
        let requester = shutdown.clone();
        let stopper = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            requester.request();
        });
        let report = run_watch(
            &config,
            &root.path().join("history.db"),
            &AutomationSelection::default(),
            &shutdown,
        )
        .unwrap();
        stopper.join().unwrap();
        assert!(!report.has_failures());
        assert!(source.join("unfinished.txt").exists());
        assert!(!destination.exists());
    }

    #[test]
    fn one_watch_failure_does_not_stop_another_directory() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("ready.txt"), b"ready").unwrap();
        let config = automation_config(
            text_rule(destination.clone()),
            vec![
                directory(
                    "missing",
                    root.path().join("does-not-exist"),
                    ExecutionMode::Watch,
                ),
                directory("healthy", source, ExecutionMode::Watch),
            ],
        );
        let shutdown = ShutdownToken::default();
        let waiter = stop_after_path(destination.join("ready.txt"), shutdown.clone());
        let report = run_watch(
            &config,
            &root.path().join("history.db"),
            &AutomationSelection::default(),
            &shutdown,
        )
        .unwrap();
        assert!(waiter.join().unwrap());
        assert_eq!(report.worker_failures, 1);
        assert_eq!(fs::read(destination.join("ready.txt")).unwrap(), b"ready");
    }

    #[test]
    fn scheduled_worker_runs_and_stops_cleanly() {
        let root = tempdir().unwrap();
        let source = root.path().join("incoming");
        let destination = root.path().join("sorted");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("ready.txt"), b"ready").unwrap();
        let config = automation_config(
            text_rule(destination.clone()),
            vec![directory("nightly", source, ExecutionMode::Scheduled)],
        );
        let shutdown = ShutdownToken::default();
        let waiter = stop_after_path(destination.join("ready.txt"), shutdown.clone());
        let report = run_scheduled(
            &config,
            &root.path().join("history.db"),
            &AutomationSelection::default(),
            &shutdown,
        )
        .unwrap();
        assert!(waiter.join().unwrap());
        assert!(!report.has_failures());
        assert_eq!(report.workers[0].executions, 1);
    }
}
