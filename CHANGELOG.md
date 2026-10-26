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

### Known limitations

- No cross-filesystem transfer or automatic recovery.
- No watcher, scheduler runtime, desktop GUI, or benchmarks.

