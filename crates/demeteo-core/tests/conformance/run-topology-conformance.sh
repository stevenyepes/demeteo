#!/usr/bin/env bash
# Topology-equivalence conformance gate (C5,
# docs/EXECUTION_CONSISTENCY_PLAN.md): run one workflow through every
# transport and assert an equivalent RunView.
#
# Usage: crates/demeteo-core/tests/conformance/run-topology-conformance.sh
#
# Legs:
#   * local            — always (no Docker), the reference RunView.
#   * desktop-over-SSH — against a throwaway loopback sshd container
#                        (reuses the C2.2 sshd image).
#   * runner           — against a demeteo-runner container (see
#                        run-runner-conformance.sh; kept separate because it
#                        additionally needs an in-container git host).
#
# This script covers the local + SSH legs. Requires Docker.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"

IMAGE="demeteo-ssh-conformance:latest"
CONTAINER="demeteo-topology-conformance-$$"
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

echo "==> Running the topology local + SSH equivalence legs"
export DEMETEO_SSH_CONFORMANCE_HOST="127.0.0.1"
export DEMETEO_SSH_CONFORMANCE_PORT="$SSH_PORT"
export DEMETEO_SSH_CONFORMANCE_USER="$SSH_USER"
export DEMETEO_SSH_CONFORMANCE_PASSWORD="$SSH_PASSWORD"

cd "$REPO_ROOT"
# The local leg (`topology_local_leg_produces_expected_runview`) runs
# unconditionally; `topology_local_matches_ssh` asserts the SSH leg renders an
# equivalent RunView.
cargo test -p demeteo-core --features ssh-conformance \
  --lib topology_ -- --nocapture
