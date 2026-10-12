# Everything is driven from the terminal: `make dev`, `make test`, `make build`.
.PHONY: install dev test test-live test-live-mariadb mariadb-up mariadb-down lint build clean

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

# A throwaway MariaDB for the live tests (needs Docker): `make mariadb-up test-live-mariadb mariadb-down`.
MARIADB_PORT ?= 33308

mariadb-up:
	docker run -d --name lfdb-mariadb -e MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=1 -p 127.0.0.1:$(MARIADB_PORT):3306 mariadb:11

mariadb-down:
	docker rm -f lfdb-mariadb

test-live-mariadb:
	cd src-tauri && LFDB_TEST_HOST=127.0.0.1 LFDB_TEST_PORT=$(MARIADB_PORT) LFDB_TEST_USER=root cargo test -- --include-ignored

lint:
	cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings

build:
	npm run tauri build

clean:
	rm -rf dist
	cd src-tauri && cargo clean
