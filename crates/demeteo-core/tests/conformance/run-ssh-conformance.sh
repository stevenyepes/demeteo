#!/usr/bin/env bash
# Stand up a throwaway loopback sshd, run the C2.2 SSH leg of the
# ExecutionPort conformance suite against it, and tear it down.
# (docs/EXECUTION_CONSISTENCY_PLAN.md)
#
# Usage: crates/demeteo-core/tests/conformance/run-ssh-conformance.sh
#
# Requires Docker. Runs the byte-identical `exec_contract` the local adapter
# passes against `SshClientAdapter` pointed at the container — the parity gate.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"

IMAGE="demeteo-ssh-conformance:latest"
CONTAINER="demeteo-ssh-conformance-$$"
SSH_USER="${DEMETEO_SSH_CONFORMANCE_USER:-demeteo}"
SSH_PASSWORD="${DEMETEO_SSH_CONFORMANCE_PASSWORD:-conformance}"
SSH_PORT="${DEMETEO_SSH_CONFORMANCE_PORT:-2222}"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> Building $IMAGE"
docker build \
  --build-arg "SSH_USER=$SSH_USER" \
  --build-arg "SSH_PASSWORD=$SSH_PASSWORD" \
  -t "$IMAGE" "$HERE/sshd"

echo "==> Starting sshd on 127.0.0.1:$SSH_PORT"
docker run -d --name "$CONTAINER" -p "127.0.0.1:$SSH_PORT:22" "$IMAGE" >/dev/null

echo "==> Waiting for sshd to accept TCP"
for i in $(seq 1 30); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$SSH_PORT") 2>/dev/null; then
    exec 3>&- 3<&-
    break
  fi
  if [ "$i" -eq 30 ]; then
    echo "sshd did not come up in time; container logs:" >&2
    docker logs "$CONTAINER" >&2 || true
    exit 1
  fi
  sleep 1
done

echo "==> Running the SSH conformance leg"
export DEMETEO_SSH_CONFORMANCE_HOST="127.0.0.1"
export DEMETEO_SSH_CONFORMANCE_PORT="$SSH_PORT"
export DEMETEO_SSH_CONFORMANCE_USER="$SSH_USER"
export DEMETEO_SSH_CONFORMANCE_PASSWORD="$SSH_PASSWORD"
export DEMETEO_SSH_CONFORMANCE_WORKDIR="/home/$SSH_USER/conformance"

cd "$REPO_ROOT"
cargo test -p demeteo-core --features ssh-conformance \
  ssh_client_adapter_satisfies_the_contract -- --nocapture
