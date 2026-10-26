# Phase 3 summary

## Implemented

- durable SQLite intent and transition journal;
- `PLANNED`, `IN_PROGRESS`, `COPIED`, `VERIFIED`, `PUBLISHED`,
  `RECOVERY_REQUIRED`, and terminal operation states;
- lossless native-path persistence on Unix and Windows;
- SQLite-backed exclusion of concurrent operations on one normalized source
  path;
- destination-side staged transfers for cross-filesystem moves;
- bounded-buffer copying, file synchronization, byte-for-byte verification,
  source fingerprint checks, and no-clobber publication;
- inspection-only `sorter recover` and explicit `sorter recover --apply`;
- conservative actual-filesystem reconciliation and idempotent terminal state;
- fault injection at copy, verification, publication, deletion, and journal
  boundaries.

## Architectural decisions

Filesystem code emits transition events without depending on SQLite. The
runtime implements those hooks and persists them through `sorter-storage`.
This keeps transfer mechanics independently testable and prevents either the
CLI or a future GUI from owning safety logic.

Verification uses a direct byte comparison rather than a persisted hash. This
costs a second sequential read for cross-filesystem moves, but avoids hash
collision assumptions and hashing work for same-filesystem moves.

Execution remains sequential. The persistent source lock establishes the
correctness prerequisite for future bounded concurrency without introducing a
worker runtime prematurely.

## Validation

The phase is validated with workspace unit, integration, property, CLI, and
fault-injection tests; strict Clippy; formatting checks; a locked release build;
Compose rendering; and the Linux/macOS/Windows GitHub Actions matrix.

## Remaining limitations

- watch and scheduled execution are Phase 4;
- actively written files require caller discipline until watch stabilization
  exists;
- hard-link publication must be supported by the destination filesystem;
- directory replacement races require platform-specific handle-relative APIs
  for further hardening;
- real removable, network, and unusual filesystems need platform test coverage;
- filesystem durability ultimately depends on OS, mount, hardware, and device
  guarantees;
- hard-link aliases can bypass path-based source locking.

## Next phase

Phase 4 adds native filesystem watching and cron scheduling with bounded event
queues, debounce, stabilization checks, temporary-file exclusions, periodic
reconciliation, overlap prevention, independent directory failure domains, and
graceful shutdown. The desktop GUI remains deferred to Phase 5.
