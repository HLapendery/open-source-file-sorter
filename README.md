# Open Source File Sorter

A conservative, headless-first file organizer written in Rust. Rules are
deterministic, existing files are never overwritten, and every real operation
is recorded in SQLite.

> **Project status:** Phase 2 CLI MVP. One-shot sorting is usable. Cross-filesystem
> transfers, crash recovery, watching, scheduling, and the desktop GUI are not
> implemented yet; see [the roadmap](docs/roadmap.md).

## What works

- exact filename, glob, regular-expression, and extension matching;
- case-sensitive or case-insensitive rules;
- `all` (AND) and `any` (OR) combinations between condition groups;
- deterministic priority and first-match-wins resolution;
- non-recursive and recursive one-shot scans;
- dry-run and JSON output;
- `skip`, `rename-with-suffix`, and `error` collision policies;
- no-clobber same-filesystem moves with data synchronization;
- SQLite operation history;
- multiple named directory configurations;
- TOML and cron syntax validation;
- Linux, macOS, and Windows compilation in CI.

## Quick start with Docker Compose

The repository wrapper supplies the normal homelab environment files:

~~~
mkdir -p file-sorter/sandbox/incoming
cp ~/Downloads/example.pdf file-sorter/sandbox/incoming/

./compose-stack file-sorter config
./compose-stack file-sorter run --rm sorter validate
./compose-stack file-sorter run --rm sorter run --directory downloads --dry-run
./compose-stack file-sorter run --rm sorter run --directory downloads
./compose-stack file-sorter run --rm sorter history
~~~

The default Compose mount maps `file-sorter/sandbox` to `/files`. To organize a
different host tree, set `SORTER_FILES_ROOT` before invoking the wrapper:

~~~
SORTER_FILES_ROOT=/srv/incoming ./compose-stack file-sorter run --rm sorter \
  run --directory downloads --dry-run
~~~

The container runs as UID 1000. The mounted configuration, database, source,
and destination must be accessible to that user. Always inspect a dry run
before the first real execution.

## Native development

Rust 1.88 or newer is required.

~~~
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p sorter-cli -- --config examples/sorter.example.toml validate
cargo run -p sorter-cli -- --config sorter.toml init
~~~

Common commands:

~~~text
sorter init
sorter validate
sorter run PATH --dry-run [--recursive] [--json] [-v]
sorter run --directory ID [--collision skip|rename-with-suffix|error]
sorter rules list [--json]
sorter rules enable ID
sorter rules disable ID
sorter directories list [--json]
sorter history [--limit 50] [--json]
~~~

Exit code `0` means success, `1` means a command/configuration error, and `3`
means the scan completed but at least one file or directory entry failed.

## Safety model

Phase 2 moves regular files only. Source symlinks, watched-root symlinks, and
destination-directory symlinks are rejected. On one filesystem, the engine
publishes a hard link with no-clobber semantics, synchronizes it, then removes
the source. If publication or synchronization fails, the source remains. If
source removal fails, both names remain and history is marked
`RECOVERY_REQUIRED`.

Hard links are not supported by every filesystem. Cross-filesystem moves are
refused in this release instead of falling back to an unsafe copy. See
[filesystem safety](docs/filesystem-safety.md) for guarantees and limitations.

## Documentation

- [Architecture and decisions](docs/architecture.md)
- [Configuration reference](docs/configuration.md)
- [Filesystem safety](docs/filesystem-safety.md)
- [Development roadmap](docs/roadmap.md)
- [Developer guide](docs/development.md)

## License

Licensed under either Apache License 2.0 or the MIT license, at your option.
