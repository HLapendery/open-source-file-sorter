# Desktop application

This directory contains the Tauri 2, Vue 3, and TypeScript desktop interface.
All matching, scanning, filesystem movement, history, and automation operations
run in Rust through commands in `src-tauri`; the Vue application only presents
state and collects user input.

## Development

Install the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/),
Node.js 20, and Rust 1.90, then run:

~~~bash
npm ci
npm run tauri dev
~~~

Frontend-only type checking and production asset compilation:

~~~bash
npm run build
~~~

Linux contributors can validate the Rust desktop crate in the same environment
as CI without installing WebKitGTK on the host:

~~~bash
docker build -f desktop/Dockerfile.builder -t open-source-file-sorter:desktop-builder .
docker run --rm -v "$PWD:/work" -w /work \
  open-source-file-sorter:desktop-builder \
  cargo test --workspace --all-targets
~~~

The application creates an empty versioned configuration on first start. Its
default location comes from Tauri's application config directory, while the
SQLite journal uses the application data directory. `SORTER_CONFIG` and
`SORTER_DATABASE` override those paths.

There are no installers or signed packages yet. Those belong to Phase 6.
