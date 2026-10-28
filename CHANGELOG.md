# Changelog

All notable changes will be documented here. This project follows Keep a
Changelog and intends to use Semantic Versioning after the first release.

## [Unreleased]

### Added

- Cargo workspace with core, filesystem, runtime, storage, and CLI crates.
- Deterministic rule matching, one-shot sorting, dry-run, and JSON output.
- Safe no-clobber same-filesystem moves and three collision policies.
- Versioned TOML configuration, cron validation, and SQLite history.
- Docker Compose stack, automated tests, CI, and initial public documentation.
- Durable operation journal with lossless native path metadata.
- Verified staged transfers across filesystem boundaries.
- Conservative, idempotent `sorter recover` inspection and recovery.
- Persistent per-source operation locking and transfer fault-injection tests.
- Native multi-directory watchdog with bounded debounce and stabilization.
- Streaming reconciliation for missed events and queue overflow.
- Independent local/UTC cron workers with overlap prevention.
- Graceful signal handling and Docker Compose automation profiles.
- Tauri 2 and Vue 3 desktop interface backed by the existing Rust engine.
- Desktop dashboard, directory/rule editing, dry-run preview, explicit
  execution, automation controls, and SQLite history.
- English-first desktop localization with a persistent French/English selector
  and locale-aware dates and operation states.
- Rust desktop backend tests and Linux/macOS/Windows desktop compilation in CI.

### Changed

- Refreshed the desktop shell, navigation, cards, forms, and responsive layout
  with a cleaner SaaS-style visual system.

### Known limitations

- No signed installers, release packages, incremental desktop scan progress,
  or benchmarks.
- Durability remains subject to operating-system, filesystem, mount, and
  hardware guarantees documented in `docs/filesystem-safety.md`.
