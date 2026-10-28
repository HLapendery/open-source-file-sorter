# Architecture

## Scope and status

This document records the architecture through the Phase 5 desktop MVP.

## Workspace boundaries

~~~text
sorter-cli
  -> sorter-runtime
       -> sorter-core
       -> sorter-fs
       -> sorter-storage

sorter-desktop (Tauri commands)
  -> sorter-runtime
  -> sorter-core
  -> sorter-storage

Vue 3 UI -> typed Tauri IPC -> sorter-desktop

sorter-storage -> sorter-core, sorter-fs (shared serialized domain types)
~~~

- `sorter-core` contains rule types, validation, compiled matchers, priority
  ordering, and move plans. It has no CLI, database, watcher, or GUI dependency.
- `sorter-fs` owns conservative filesystem mutations and collision policies.
- `sorter-storage` owns versioned TOML configuration and SQLite history.
- `sorter-runtime` streams directory entries and coordinates one-shot, watch,
  scheduled, stabilization, reconciliation, and shutdown work.
- `sorter-cli` translates commands and output formats. It contains no matching
  or movement logic.
- `sorter-desktop` exposes narrow asynchronous Tauri commands. Blocking Rust
  scans and movements run outside the UI thread.
- `desktop/src` renders and edits state. It never matches or moves files.

The CLI and desktop process call the same crates directly. There is no local
HTTP service and no duplicated TypeScript domain implementation.

## Major decisions and trade-offs

### Rust workspace and synchronous I/O

Filesystem operations are synchronous because standard filesystem APIs are
blocking. This avoids an async runtime and keeps failure ordering
understandable. One worker owns each automated directory, scheduled runs cannot
overlap themselves, and a small global gate bounds work across directories.
SQLite additionally enforces one active operation per normalized source path
across processes.

### Native events with reconciliation

`notify` selects the native watcher backend. Its callback only attempts to
enqueue an event in a bounded channel; it never performs filesystem work.
Workers debounce paths and require two unchanged observations plus a configured
stabilization interval before matching or moving them.

Native notifications are hints because operating systems and network
filesystems can omit them. A periodic `walkdir` iterator supplies
reconciliation candidates into the same bounded pending set. The iterator is
retained between batches, so a directory with hundreds of thousands of files
does not require an equally large in-memory path list. Queue overflow requests
another reconciliation.

### Scheduling and shutdown

One thread owns each cron-configured directory and calculates occurrences in
the explicitly selected local or UTC timezone. It completes a run before
asking for the next occurrence. A shared condition-variable token wakes
schedulers for `SIGINT`, `SIGTERM`, or `SIGHUP` and watcher loops stop at safe
boundaries.

### Compiled, deterministic matching

Glob and regex patterns compile once when an engine is built. Enabled rules are
sorted by descending priority. Equal priorities retain TOML declaration order.
Only the first match produces a plan, so one source file can never receive
conflicting moves in a single run.

A match specification contains condition groups. Values inside one group are
ORed; `operator = "all"` ANDs the populated groups, while `operator = "any"`
ORs them. New groups can be added without changing rule ordering or planning.

### No-clobber publication and staged transfer

Portable `std::fs::rename` may replace an existing destination. The engine
therefore publishes with `hard_link`, which is atomic and fails if the final
name already exists. On one filesystem this directly links the source. Across
filesystems, data is streamed into a private file beside the destination,
synchronized, compared byte-for-byte with the still-open source, and only then
published. The source is revalidated before deletion.

Hard links are used as the portable no-clobber primitive. Filesystems without
hard-link support fail closed. Copy verification deliberately avoids storing a
hash: a direct comparison costs a second read but has no collision assumption
and no persistent checksum overhead.

### SQLite with bundled SQLite

SQLite provides durable local history without a service. WAL plus `FULL`
synchronous mode is enabled. The bundled feature increases binary size and
compile time but makes desktop and container installations predictable.

Each transfer records its selected destination, private temporary path, kind,
source fingerprint, and state before destructive work. States include
`PLANNED`, `IN_PROGRESS`, `COPIED`, `VERIFIED`, `PUBLISHED`,
`RECOVERY_REQUIRED`, and terminal results. Recovery treats the filesystem as
authoritative, verifies content or file identity, and refuses ambiguous
deletion. `sorter recover` is inspection-only unless `--apply` is explicit.

### TOML configuration with explicit versioning

TOML is readable and works well for declarative rules. Unknown fields are
rejected to catch typos. `version = 1` is mandatory semantically, and unsupported
versions fail closed. Future migrations belong in `sorter-storage`.

### Headless-first delivery and native desktop boundary

The CLI remains the primary product surface and is fully usable in headless
environments. Tauri 2 provides a thin native window and Vue 3 provides the
presentation layer. Commands accept configuration-shaped data, validate it in
Rust, and call the existing runtime. Long-running calls use Tauri's blocking
worker pool so the window remains responsive.

Watch and scheduled workers are owned by a Rust automation manager. Each has a
shutdown token and join handle; configuration cannot be saved while workers are
active, and application shutdown requests every worker to stop before joining
it. Preview is a real runtime dry run. Execution deliberately rescans and
revalidates instead of trusting a stale frontend plan.

Configuration and SQLite paths default to Tauri's per-platform application
directories. Environment overrides allow the desktop process to share an
explicit CLI configuration. Native directory selection is the only frontend
capability beyond core IPC.

The GUI is not containerized: a desktop window must integrate with the host's
window manager, webview, and directory picker. Compose remains the simple
headless installation path. Native installers, signing, and packaging are
deferred to Phase 6.

## Risks

- stabilization cannot prove that a paused external writer has permanently
  finished, and native events have platform/filesystem-specific limitations;
- directory replacement races, hard-link aliases, mount points, network
  filesystems, and case folding vary by platform;
- source-change detection uses identity, size, modification time, and
  byte-for-byte verification; no portable userspace API can prevent a hostile
  writer in the final validation-to-unlink interval;
- high-volume throughput must be measured before concurrency is introduced.
- one-shot desktop scans currently return a complete report rather than
  streaming incremental rows and cannot yet be cancelled mid-scan;
- the native UI depends on the operating system webview and has not yet been
  distributed as a signed installer.

## Phase summaries

### Phase 1 — Architecture

Implemented the workspace boundaries above, selected sequential synchronous
execution for the MVP, documented safety trade-offs, and defined the staged
roadmap.

### Phase 2 — Functional CLI MVP

Implemented configuration loading/validation, multiple directory definitions,
compiled rules, deterministic planning, dry runs, safe same-filesystem moves,
collision policies, SQLite history, human/JSON CLI output, and automated unit,
property, integration, and CLI tests.

Known limitations are listed in the roadmap and filesystem safety document.

### Phase 3 — Reliability

Implemented a durable SQLite journal with lossless native path storage,
cross-filesystem staged transfers, byte-for-byte verification, source-change
checks, persistent per-source locking, conservative idempotent recovery, and
fault-injection coverage around copy, publication, source deletion, and journal
boundaries.

### Phase 4 — Automation

Implemented native watchdog workers, bounded event handling, debounce,
stabilization, temporary-file exclusions, streaming reconciliation, local/UTC
cron workers, global bounded execution, independent directory failure domains,
and graceful signal-driven shutdown. Compose profiles run watch and schedule
workers without adding a daemon or HTTP API.

### Phase 5 — Desktop MVP

Implemented the Tauri 2 shell, Vue 3 dashboard, directory and rule editors,
Rust-backed filename testing, native path selection, dry-run preview, explicit
one-shot execution, automation lifecycle controls, and SQLite history. Desktop
backend tests exercise first-start initialization, non-mutating preview, and
journaled execution. Cross-platform CI compiles both CLI and desktop targets.
