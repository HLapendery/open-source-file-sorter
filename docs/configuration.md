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
flag, `mode`, and an ordered list of rule IDs. The current executable supports
`mode = "once"` only. `watch` and `scheduled` can be represented but are not
executed until Phase 4.

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
default or may be `utc`. This is a configuration contract for Phase 4; no
scheduler runs in the current release. `local` will mean the process/container
timezone (the Compose stack supplies `TZ`), including daylight-saving changes.
`utc` will avoid daylight-saving ambiguity. A scheduled run will never overlap
another run for the same directory.

## Collision policy

- `skip`: leave both paths unchanged and record a skipped operation;
- `rename-with-suffix`: choose `name (1).ext`, then the next available suffix;
- `error`: preserve both files and report a per-file failure.

Overwrite is intentionally unavailable.

See [the example](../examples/sorter.example.toml) for a complete file.

