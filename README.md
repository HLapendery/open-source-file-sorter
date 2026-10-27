# Open Source File Sorter

A conservative, headless-first file organizer written in Rust. Rules are
deterministic, existing files are never overwritten, and every real operation
is recorded in SQLite.

> **Project status:** Phase 4 headless automation. One-shot sorting, verified
> cross-filesystem transfer, durable recovery, native watching, and cron
> scheduling are usable. The desktop GUI is not implemented yet; see
> [the roadmap](docs/roadmap.md).

## What works

- exact filename, glob, regular-expression, and extension matching;
- case-sensitive or case-insensitive rules;
- `all` (AND) and `any` (OR) combinations between condition groups;
- deterministic priority and first-match-wins resolution;
- non-recursive and recursive one-shot scans;
- dry-run and JSON output;
- `skip`, `rename-with-suffix`, and `error` collision policies;
- no-clobber same- and cross-filesystem moves with synchronization and
  byte-for-byte copy verification;
- durable SQLite operation journal, history, and lossless native path metadata;
- dry-run recovery inspection and explicit, idempotent `--apply` recovery;
- persistent protection against simultaneous operations on one source path;
- multiple named directory configurations;
- TOML and cron syntax validation;
- independent native filesystem watchers with bounded queues;
- debounce, file stabilization, temporary-file exclusions, and reconciliation;
- independent local/UTC cron schedules with overlap prevention;
- graceful `SIGINT`/`SIGTERM` shutdown;
- Linux, macOS, and Windows compilation in CI.

## Installation with Docker Compose

Docker Engine with the Compose plugin, or Docker Desktop on macOS/Windows, is
the only installation prerequisite.

~~~bash
git clone https://github.com/HLapendery/open-source-file-sorter.git
cd open-source-file-sorter

mkdir -p config data sandbox/incoming
cp examples/sorter.example.toml config/sorter.toml
docker compose build
docker compose run --rm sorter validate
~~~

PowerShell equivalent after cloning:

~~~powershell
New-Item -ItemType Directory -Force config, data, sandbox/incoming
Copy-Item examples/sorter.example.toml config/sorter.toml
docker compose build
docker compose run --rm sorter validate
~~~

Place a test file in `sandbox/incoming`, preview the plan, then run it:

~~~bash
cp ~/Downloads/example.pdf sandbox/incoming/
docker compose run --rm sorter run --directory downloads --dry-run
docker compose run --rm sorter run --directory downloads
docker compose run --rm sorter history
docker compose run --rm sorter recover
~~~

The example configuration declares `downloads` in `watch` mode. Start the
long-running watcher profile with:

~~~bash
docker compose --profile watch up -d watcher
docker compose logs -f watcher
docker compose stop watcher
~~~

For scheduled directories, set `mode = "scheduled"` and a six-field cron
expression in `config/sorter.toml`, then start:

~~~bash
docker compose --profile schedule up -d scheduler
docker compose logs -f scheduler
docker compose stop scheduler
~~~

The homelab wrapper remains optional. From the parent homelab repository, the
equivalent commands are:

~~~bash
mkdir -p file-sorter/sandbox/incoming
./compose-stack file-sorter config
./compose-stack file-sorter run --rm sorter validate
~~~

The default Compose mount maps `file-sorter/sandbox` to `/files`. To organize a
different host tree, set `SORTER_FILES_ROOT` before invoking the wrapper:

~~~
SORTER_FILES_ROOT=/srv/files docker compose run --rm sorter \
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
sorter watch [--directory ID] [--json]
sorter schedule [--directory ID] [--json]
sorter rules list [--json]
sorter rules enable ID
sorter rules disable ID
sorter directories list [--json]
sorter history [--limit 50] [--json]
sorter recover [--operation ID] [--apply] [--json]
~~~

Exit code `0` means success, `1` means a command/configuration error, and `3`
means the scan completed but at least one file or directory entry failed.
Recovery returns `4` when at least one operation requires manual review.

## Safety model

The engine moves regular files only. Source symlinks, watched-root symlinks, and
destination-directory symlinks are rejected. On one filesystem, the engine
publishes a hard link with no-clobber semantics, synchronizes it, then removes
the source. Across filesystems it copies to a private destination-side file,
synchronizes and compares it byte-for-byte, publishes without clobbering, and
only then removes the source.

Every mutation is journaled. Recovery inspects both the journal and actual
paths, deletes a source only after renewed verification, and leaves ambiguous
states untouched for manual review. See
[filesystem safety](docs/filesystem-safety.md) for guarantees and limitations.

Native filesystem notification is an optimization, not the source of truth:
bounded periodic reconciliation finds eligible files after dropped or
unsupported events. A watcher requires unchanged size and modification time
for the configured stabilization interval, but this remains a heuristic and
cannot prove that an external writer has permanently closed a file.

## Documentation

- [Architecture and decisions](docs/architecture.md)
- [Configuration reference](docs/configuration.md)
- [Filesystem safety](docs/filesystem-safety.md)
- [Phase 3 implementation summary](docs/phase-3-summary.md)
- [Phase 4 implementation summary](docs/phase-4-summary.md)
- [Development roadmap](docs/roadmap.md)
- [Developer guide](docs/development.md)

## License

Licensed under either Apache License 2.0 or the MIT license, at your option.
