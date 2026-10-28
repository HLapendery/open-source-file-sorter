export type CollisionPolicy = "skip" | "rename-with-suffix" | "error";
export type ExecutionMode = "once" | "watch" | "scheduled";
export type ScheduleTimezone = "local" | "utc";
export type MatchOperator = "all" | "any";

export interface Settings {
  collision_policy: CollisionPolicy;
  watch_debounce_ms: number;
  stabilization_delay_ms: number;
  reconciliation_interval_seconds: number;
  event_queue_capacity: number;
  max_concurrent_operations: number;
  temporary_file_globs: string[];
}

export interface MatchSpec {
  operator: MatchOperator;
  exact_filenames: string[];
  filename_globs: string[];
  filename_regexes: string[];
  extensions: string[];
  case_sensitive: boolean;
}

export interface Rule {
  id: string;
  enabled: boolean;
  priority: number;
  destination: string;
  match: MatchSpec;
}

export interface DirectoryConfig {
  id: string;
  path: string;
  enabled: boolean;
  mode: ExecutionMode;
  recursive: boolean;
  schedule: string | null;
  schedule_timezone: ScheduleTimezone;
  rules: string[];
}

export interface Config {
  version: number;
  settings: Settings;
  rules: Rule[];
  directories: DirectoryConfig[];
}

export interface HistoryEntry {
  id: number;
  source: string;
  destination: string;
  temporary_path: string | null;
  rule_id: string;
  state: string;
  transfer_kind: string | null;
  error: string | null;
  created_at_unix_ms: number;
  updated_at_unix_ms: number;
}

export interface AutomationStatus {
  directory_id: string;
  mode: string;
  state: "running" | "stopping" | "completed" | "failed";
  started_at_unix_ms: number;
  executions: number;
  item_failures: number;
  error: string | null;
}

export interface DesktopSnapshot {
  config: Config;
  history: HistoryEntry[];
  config_path: string;
  database_path: string;
  automation: AutomationStatus[];
}

export interface PlannedMove {
  source: string;
  destination: string;
  rule_id: string;
  priority: number;
}

export interface ItemReport {
  source: string;
  destination: string;
  rule_id: string;
  status: "planned" | "moved" | "skipped_collision" | "failed";
  error?: string;
}

export interface RunReport {
  source: string;
  dry_run: boolean;
  scanned: number;
  matched: number;
  moved: number;
  skipped: number;
  failed: number;
  items: ItemReport[];
  scan_errors: string[];
}

export type ViewName = "dashboard" | "directories" | "rules" | "preview" | "history";
