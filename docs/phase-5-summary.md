# Phase 5 implementation summary

## Delivered

- Tauri 2 desktop crate inside the Cargo workspace;
- Vue 3 and TypeScript dashboard with responsive navigation;
- directory creation, editing, enablement, execution modes, schedules, rule
  associations, and native folder selection;
- rule creation, matching conditions, priorities, destinations, enablement,
  and Rust-backed filename testing;
- non-mutating one-shot preview and explicit journaled execution;
- watch and scheduled worker start, status polling, and graceful stop;
- SQLite history and failure display;
- first-start configuration creation and platform-native app paths;
- frontend and Rust desktop builds in cross-platform CI.

## Architectural decisions

The desktop crate is an adapter over `sorter-core`, `sorter-runtime`, and
`sorter-storage`. Tauri commands move blocking work onto worker threads. Vue
never performs matching, scanning, movement, recovery, or journal operations.

Preview and execution are separate runtime calls. Execution rescans the source
so it never trusts a stale plan; the filesystem layer performs collision and
source validation again at mutation time. Configuration saves are rejected
while automation workers are active.

The GUI is a native host application, not a Compose service. Compose remains
the supported minimal headless installation workflow.

## Validation

The desktop backend tests cover empty first-start initialization, matching and
non-mutating preview, and journaled execution. CI runs Rust formatting, Clippy,
the complete workspace tests, frontend type checking/build, CLI builds, and
desktop builds on Linux, Windows, and macOS.

## Remaining limitations

- no signed installer or release package yet;
- one-shot scans return a complete report after the scan instead of streaming
  incremental rows;
- active one-shot scans do not yet expose cancellation;
- automation status reports completed-run totals rather than per-file live
  progress;
- packaging, benchmarks, accessibility review, and visual regression coverage
  remain Phase 6 work.

## Next phase

Phase 6 will focus on measurable benchmarks, packaging and signing, platform
limitation documentation, security review, and release automation. Installer
work should begin only after validating the native application on real Linux,
Windows, and macOS hosts.
