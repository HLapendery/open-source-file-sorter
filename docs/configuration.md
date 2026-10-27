# Configuration reference

Configuration is TOML with `version = 1`. Unknown keys and unknown referenced
rule IDs are errors.

## Matching

Supported groups are:

- `exact_filename` or `exact_filenames`;
- `filename_glob` or `filename_globs`;
- `filename_regex` or `filename_regexes`;
- `extension` or `extensions`.

Singular keys accept one string and plural keys accept arrays. Values in a
group are alternatives. Populated groups are combined with `operator = "all"`
(the default) or `operator = "any"`. Matching is case-insensitive by default;
set `case_sensitive = true` per rule when required.

Rules are evaluated by descending numeric priority. Ties use declaration order.
The first matching enabled rule wins.

## Directories

Each directory has a unique `id`, source `path`, `enabled` flag, `recursive`
flag, `mode`, and an ordered list of rule IDs. All three modes are executable:

- `once` through `sorter run`;
- `watch` through `sorter watch`;
- `scheduled` through `sorter schedule`.

Without `--directory`, watch and schedule start one independent worker for
every enabled directory in their respective mode. A fatal error in one worker
is reported without stopping the others.

Recursive configurations are rejected when a rule destination is lexically
inside the source tree.

## Cron expressions and timezones

Scheduled configurations require a valid cron expression. Six-field expressions
include seconds:

~~~text
second minute hour day-of-month month day-of-week
0      0      2    *            *     *
~~~

The parser also accepts the optional year field and standard ranges, lists, and
steps supported by the Rust `cron` crate. `schedule_timezone` is `local` by
default or may be `utc`. `local` means the process/container timezone (the
Compose stack supplies `TZ`), including daylight-saving transitions. `utc`
avoids daylight-saving ambiguity. Each directory has one synchronous schedule
worker, so a run never overlaps a previous run for that directory.

## Automation settings

The global `[settings]` table accepts:

- `watch_debounce_ms`: coalescing delay after the last native event;
- `stabilization_delay_ms`: time for which size and modification time must
  remain unchanged;
- `reconciliation_interval_seconds`: interval between streaming scans that
  recover missed events;
- `event_queue_capacity`: bound shared by the native event channel and pending
  stabilization set;
- `max_concurrent_operations`: global bound across directory workers;
- `temporary_file_globs`: filename patterns ignored by watchers, reconciliation,
  and scheduled scans.

Defaults are shown in `examples/sorter.example.toml`. Queue overflow does not
drop safety guarantees: it requests a reconciliation scan. Reconciliation
streams directory entries into the same bounded stabilization set rather than
materializing an entire large directory.

Stabilization is necessarily heuristic. A file may remain unchanged during the
delay while its writer still intends to resume later. Temporary patterns and a
conservative delay should be configured for the download/application behavior
in use.

## Collision policy

- `skip`: leave both paths unchanged and record a skipped operation;
- `rename-with-suffix`: choose `name (1).ext`, then the next available suffix;
- `error`: preserve both files and report a per-file failure.

Overwrite is intentionally unavailable.

See [the example](../examples/sorter.example.toml) for a complete file.
