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

### Known limitations

- No watcher, scheduler runtime, desktop GUI, or benchmarks.
- Durability remains subject to operating-system, filesystem, mount, and
  hardware guarantees documented in `docs/filesystem-safety.md`.
