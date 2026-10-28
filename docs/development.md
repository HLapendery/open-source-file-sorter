# Developer guide

The project uses Rust 1.90 and a Cargo workspace. Keep domain logic out of the
CLI, Tauri commands, and TypeScript frontend.

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

The desktop frontend requires Node.js 20. After installing the native
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/):

~~~bash
cd desktop
npm ci
npm run build
npm run tauri dev
~~~

On Linux, use the checked-in builder when WebKitGTK development packages are
not installed on the host:

~~~bash
docker build -f desktop/Dockerfile.builder \
  -t open-source-file-sorter:desktop-builder .
docker run --rm -v "$PWD:/work" -w /work \
  open-source-file-sorter:desktop-builder \
  cargo test --workspace --all-targets
~~~

Standalone Compose validation from this repository:

~~~bash
docker compose config
docker compose build
docker compose run --rm sorter validate
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
