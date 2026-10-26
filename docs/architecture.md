# Architecture

## Scope and status

This document records the Phase 1 decisions and the implemented Phase 2 CLI
MVP. It intentionally does not claim the reliability features planned for
later phases.

## Workspace boundaries

~~~text
sorter-cli
  -> sorter-runtime
       -> sorter-core
       -> sorter-fs
       -> sorter-storage

sorter-storage -> sorter-core, sorter-fs (shared serialized domain types)
~~~

- `sorter-core` contains rule types, validation, compiled matchers, priority
  ordering, and move plans. It has no CLI, database, watcher, or GUI dependency.
- `sorter-fs` owns conservative filesystem mutations and collision policies.
- `sorter-storage` owns versioned TOML configuration and SQLite history.
- `sorter-runtime` streams directory entries and coordinates one-shot work.
- `sorter-cli` translates commands and output formats. It contains no matching
  or movement logic.

The future Tauri process will call these crates directly. TypeScript will only
render state and invoke Rust commands.

## Major decisions and trade-offs

### Rust workspace and synchronous I/O

Filesystem operations are synchronous because standard filesystem APIs are
blocking and the MVP executes sequentially. This avoids an async runtime and
keeps failure ordering understandable. Phase 3 will add a small bounded worker
pool only after operation locking and recovery are in place.

### Compiled, deterministic matching

Glob and regex patterns compile once when an engine is built. Enabled rules are
sorted by descending priority. Equal priorities retain TOML declaration order.
Only the first match produces a plan, so one source file can never receive
conflicting moves in a single run.

A match specification contains condition groups. Values inside one group are
ORed; `operator = "all"` ANDs the populated groups, while `operator = "any"`
ORs them. New groups can be added without changing rule ordering or planning.

### No-clobber local movement

Portable `std::fs::rename` may replace an existing destination depending on the
platform. Phase 2 therefore uses `hard_link(source, destination)`, `fsync`, then
`remove_file(source)`. Creating the destination link is atomic and fails if the
name exists. The trade-off is that filesystems without hard-link support and
cross-device moves are rejected. Platform-specific atomic rename APIs remain a
possible later optimization, but not a requirement.

### SQLite with bundled SQLite

SQLite provides durable local history without a service. WAL plus `FULL`
synchronous mode is enabled. The bundled feature increases binary size and
compile time but makes desktop and container installations predictable.

The Phase 2 state transitions are history, not yet a complete recovery journal.
Phase 3 must add durable intent metadata, actual-state reconciliation,
idempotent recovery, and fault injection before crash safety is claimed.

### TOML configuration with explicit versioning

TOML is readable and works well for declarative rules. Unknown fields are
rejected to catch typos. `version = 1` is mandatory semantically, and unsupported
versions fail closed. Future migrations belong in `sorter-storage`.

### Headless-first delivery

The CLI is the first product surface. Tauri 2 and Vue 3 remain appropriate for
the desktop phase because they reuse a native Rust backend and avoid shipping
filesystem logic in JavaScript. They are deliberately absent from the current
dependency graph.

## Risks

- crash points after a filesystem mutation but before a SQLite update need a
  true recovery journal and fault-injection tests;
- cross-filesystem copy verification must detect source changes and ambiguous
  post-crash states without deleting either copy;
- watcher events do not imply write completion and require stabilization,
  debouncing, reconciliation scans, and loop protection;
- path aliases, mount points, network filesystems, case folding, and non-UTF-8
  history representation vary by platform;
- high-volume throughput must be measured before concurrency is introduced.

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

