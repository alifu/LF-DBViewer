# Everything is driven from the terminal: `make dev`, `make test`, `make build`.
.PHONY: install dev test test-live lint build clean

install:
	npm install

dev:
	npm run tauri dev

test:
	npm test
	cd src-tauri && cargo test

# Also runs the tests that need a real server. Override LFDB_TEST_* to point elsewhere.
test-live:
	cd src-tauri && LFDB_TEST_HOST=$${LFDB_TEST_HOST:-127.0.0.1} LFDB_TEST_USER=$${LFDB_TEST_USER:-root} cargo test -- --include-ignored

lint:
	cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings

build:
	npm run tauri build

clean:
	rm -rf dist
	cd src-tauri && cargo clean
