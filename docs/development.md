# Developer guide

The project uses Rust 1.88 and a Cargo workspace. Keep domain logic out of the
CLI and, later, out of Tauri commands.

Development is pull-request based. Start from `main`, create a focused
`feat/*`, `fix/*`, `docs/*`, or `chore/*` branch, and open a PR back to
`main`. Direct feature commits to `main` are reserved for repository bootstrap
or exceptional recovery.

~~~bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --locked --release --bin sorter
~~~

Tests use temporary directories. Never point tests at personal data. Filesystem
tests must assert preservation of both the source and any existing destination
on failure.

Before changing movement semantics, enumerate crash points and define what
exists at each path after every possible failure. New reliability claims require
fault-injection coverage.

For the homelab stack:

~~~bash
cd ..
./compose-stack file-sorter config
./compose-stack file-sorter build
~~~
