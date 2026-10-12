#!/usr/bin/env bash
# A throwaway SSH server and MariaDB for the SSH tunnel tests (needs Docker and ssh-keygen).
#   scripts/ssh-test-env.sh up      start both, with port forwarding refused
#   scripts/ssh-test-env.sh allow   let the SSH server forward ports
#   scripts/ssh-test-env.sh down    remove everything
set -euo pipefail

DIR="${LFDB_SSH_DIR:-${TMPDIR:-/tmp}/lfdb-ssh-test}"
NET=lfdb-ssh-net
SSH_PORT="${LFDB_SSH_PORT:-33322}"

case "${1:-}" in
  up)
    mkdir -p "$DIR"
    # id_test has a passphrase, id_plain has none and is trusted, id_unknown is not trusted.
    [ -f "$DIR/id_test" ] || ssh-keygen -q -t ed25519 -N keypass -C lfdb-test -f "$DIR/id_test"
    [ -f "$DIR/id_plain" ] || ssh-keygen -q -t ed25519 -N '' -C lfdb-plain -f "$DIR/id_plain"
    [ -f "$DIR/id_unknown" ] || ssh-keygen -q -t ed25519 -N '' -C lfdb-unknown -f "$DIR/id_unknown"
    docker network create "$NET" >/dev/null 2>&1 || true
    docker rm -f lfdb-ssh-db lfdb-ssh >/dev/null 2>&1 || true
    docker run -d --name lfdb-ssh-db --network "$NET" -e MARIADB_ALLOW_EMPTY_ROOT_PASSWORD=1 mariadb:11 >/dev/null
    docker run -d --name lfdb-ssh --network "$NET" -p "127.0.0.1:$SSH_PORT:2222" \
      -e PUID=1000 -e PGID=1000 -e PASSWORD_ACCESS=true -e USER_NAME=tunnel -e USER_PASSWORD=tunnelpass \
      -e "PUBLIC_KEY=$(cat "$DIR/id_test.pub")" lscr.io/linuxserver/openssh-server:latest >/dev/null
    for _ in $(seq 1 60); do
      docker logs lfdb-ssh-db 2>&1 | grep -q 'ready for connections.*port: 3306' && docker logs lfdb-ssh 2>&1 | grep -q 'done\.' && break
      sleep 2
    done
    docker exec -i lfdb-ssh sh -c 'cat >> /config/.ssh/authorized_keys' < "$DIR/id_plain.pub"
    echo "ready: keys in $DIR, SSH on 127.0.0.1:$SSH_PORT (forwarding refused)"
    ;;
  allow)
    docker exec lfdb-ssh sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /config/sshd/sshd_config
    docker restart lfdb-ssh >/dev/null
    sleep 6
    echo "forwarding allowed"
    ;;
  down)
    docker rm -f lfdb-ssh lfdb-ssh-db >/dev/null 2>&1 || true
    docker network rm "$NET" >/dev/null 2>&1 || true
    echo "removed"
    ;;
  *)
    echo "usage: $0 up|allow|down" >&2
    exit 2
    ;;
esac
