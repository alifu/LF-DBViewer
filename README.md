# LF-DBViewer

A light MySQL/MariaDB client for macOS, built with Tauri 2: Rust does the database work,
the interface is plain HTML, CSS and JavaScript.

## Develop

Requirements: Rust (the pinned toolchain in `rust-toolchain.toml` installs itself through
rustup) and Node.js.

```bash
make install   # once
make dev       # run the app with live reload
make test      # JavaScript and Rust unit tests
make test-live # also the tests that need a MySQL/MariaDB server on 127.0.0.1
make lint      # rustfmt and clippy
make build     # release build of LF-DBViewer.app
```

## Layout

- `src/`: interface, grouped by feature (`features/`), with pure state in `state/` and helpers in `lib/`.
- `src-tauri/src/connection/`: connection pool, SSH tunnel, MySQL client option files.
- `src-tauri/src/storage/`: saved connections (JSON) and passwords (macOS Keychain).
- `src-tauri/src/commands/`: the thin layer the interface calls.
