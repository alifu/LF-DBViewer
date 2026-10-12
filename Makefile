# Everything is driven from the terminal: `make dev`, `make test`, `make build`.
.PHONY: install dev test test-live test-live-mariadb test-ssh mariadb-up mariadb-down lint build clean

install:
	npm install

dev:
	npm run tauri dev

test:
	npm test
	cd src-tauri && cargo test

# Also runs the tests that need a real server. Override LFDB_TEST_* to point elsewhere.
test-live:
	cd src-tauri && LFDB_TEST_HOST=$${LFDB_TEST_HOST:-127.0.0.1} LFDB_TEST_USER=$${LFDB_TEST_USER:-root} cargo test -- --include-ignored --skip ssh_tests

# A throwaway MariaDB for the live tests (needs Docker): `make mariadb-up test-live-mariadb mariadb-down`.
MARIADB_PORT ?= 33308

mariadb-up:
	docker run -d --name lfdb-mariadb -e MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=1 -p 127.0.0.1:$(MARIADB_PORT):3306 mariadb:11

mariadb-down:
	docker rm -f lfdb-mariadb

test-live-mariadb:
	cd src-tauri && LFDB_TEST_HOST=127.0.0.1 LFDB_TEST_PORT=$(MARIADB_PORT) LFDB_TEST_USER=root cargo test -- --include-ignored --skip ssh_tests

# Real SSH tunnel tests against an OpenSSH server in Docker (agent sign-in included). Needs Docker.
test-ssh:
	scripts/ssh-test-env.sh up
	@set -e; DIR=$${LFDB_SSH_DIR:-$${TMPDIR:-/tmp}/lfdb-ssh-test}; export LFDB_SSH_DIR=$$DIR LFDB_SSH_TARGET_HOST=lfdb-ssh-db; \
	  cd src-tauri && cargo test ssh_tests::a_server_that_refuses -- --include-ignored; cd ..; \
	  scripts/ssh-test-env.sh allow; \
	  eval "$$(ssh-agent -s)" >/dev/null; ssh-add -q $$DIR/id_plain; \
	  (cd src-tauri && cargo test ssh_tests -- --include-ignored --skip a_server_that_refuses); status=$$?; \
	  ssh-agent -k >/dev/null; scripts/ssh-test-env.sh down; exit $$status

lint:
	cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings

build:
	npm run tauri build

clean:
	rm -rf dist
	cd src-tauri && cargo clean
