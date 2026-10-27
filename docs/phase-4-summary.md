# Phase 4 summary

## Implemented

- `sorter watch` for all enabled watch-mode directories or one selected ID;
- native cross-platform events through `notify`;
- bounded callback queues and bounded pending stabilization state;
- debounce and configurable size/mtime stabilization;
- configurable temporary-file glob exclusions;
- streaming periodic reconciliation for missed events and queue overflow;
- destination filtering and existing recursive-loop validation;
- `sorter schedule` with independent six-field cron schedules;
- explicit local or UTC timezone evaluation;
- one synchronous worker per directory, preventing self-overlap;
- a global configurable execution bound across directory workers;
- independent worker failure reporting;
- cooperative `SIGINT`, `SIGTERM`, and `SIGHUP` shutdown;
- standalone Docker Compose instructions and watch/schedule profiles.

## Architectural decisions

Automation uses standard threads and blocking filesystem APIs. A native watcher
callback only performs a bounded non-blocking send. File inspection, rule
matching, journaling, and movement remain in Rust runtime/core components and
reuse the Phase 3 transfer path.

Reconciliation retains an owned directory iterator and feeds only as many paths
as the pending set can hold. This preserves bounded memory without repeatedly
starving later entries in a very large directory.

Scheduled work remains synchronous per directory. This makes non-overlap a
structural property rather than a distributed lock timeout.

## Validation

Tests cover stabilization resets, bounded pending state, temporary exclusions,
shutdown wakeup, reconciliation of pre-existing files, native watch execution,
independent directory failure, scheduled execution, and all Phase 2/3 safety
behavior. CI compiles release binaries on Linux, macOS, and Windows.

## Remaining limitations

- stabilization is heuristic and cannot lock out an external writer;
- native notifications may be unavailable or incomplete on network,
  virtualized, or unusual filesystems; reconciliation mitigates missed events;
- scheduled scans do not wait for watch-style stabilization and should run
  during a configured quiet period;
- runtime status is available through structured logs and the final shutdown
  report; no daemon control socket or HTTP API exists;
- the desktop GUI remains Phase 5.

## Next phase

Phase 5 adds the Tauri 2 and Vue 3 desktop application. It will call the
existing Rust engine/runtime directly for configuration, preview, execution,
monitoring, and history. TypeScript will not perform filesystem operations.
