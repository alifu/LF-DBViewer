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

## What it does

- Connect to MySQL and MariaDB, directly or through an SSH tunnel; passwords are kept in the macOS Keychain.
- Browse and edit table data (big tables stream in as you scroll), run SQL with a multi-result editor.
- Design tables, indexes and foreign keys, edit views, routines, triggers and events, with the SQL shown before it runs.
- Export a database to a SQL file that loads back identically, and import SQL files of any size.
- Light and dark themes (Settings, or ⌘,).

## Layout

- `src/`: interface, grouped by feature (`features/`), with pure state in `state/` and helpers in `lib/`.
- `src-tauri/src/connection/`: connection pool, SSH tunnel, MySQL client option files.
- `src-tauri/src/storage/`: saved connections (JSON) and passwords (macOS Keychain).
- `src-tauri/src/commands/`: the thin layer the interface calls.
