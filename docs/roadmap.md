# Roadmap

## Phase 2 — CLI MVP (implemented)

- versioned TOML configuration;
- deterministic compiled matching and one-shot planning;
- dry-run, human output, and JSON output;
- conservative same-filesystem movement and collision policies;
- basic SQLite history;
- unit, property, integration, and CLI tests.

## Phase 3 — Reliability (implemented)

- durable operation journal with transactional state transitions;
- staged cross-filesystem transfers with byte-for-byte verification;
- source identity/change checks around long copies;
- idempotent recovery based on journal plus actual filesystem state;
- SQLite-backed per-source operation locking across processes;
- fault injection at every copy, publication, deletion, and journal boundary;
- lossless persistence for native non-UTF-8 paths where supported.

Remaining hardening work includes testing on real removable/network
filesystems, platform-specific directory-handle APIs, and benchmarking before
introducing bounded parallel transfers.

## Phase 4 — Automation (implemented)

- native `notify` watcher;
- bounded event queues, debounce, stabilization, and temporary-file exclusions;
- periodic reconciliation scans and event-storm handling;
- per-directory cron scheduler with local/UTC semantics;
- overlap prevention and graceful shutdown;
- independent failure domains for all configured directories.

The implementation uses one synchronous worker per directory, a global
execution gate, bounded native-event and stabilization queues, and streaming
reconciliation. No background HTTP service or async runtime is required.

## Phase 5 — Desktop

- Tauri 2 shell with Vue 3 and TypeScript;
- dashboard, directory/rule editors, preview, live execution, and history;
- cancellable background commands and incremental result delivery;
- no business logic or filesystem mutation in TypeScript.

## Phase 6 — Release

- matching, scanning, large-directory, one-shot, and watcher-idle benchmarks;
- installers and signed artifacts where practical;
- security review and release automation;
- platform-specific durability and watcher documentation.
