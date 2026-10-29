#!/usr/bin/env bash
# One-command reproduction of the libssh2 + OpenSSH user-certificate spike
# (docs/HUB_SSH_CERT_SPIKE.md). Generates a throwaway CA and keys, starts a real
# sshd that trusts the CA, starts ssh-agents holding key+cert, runs the
# #[ignore]d `ssh_cert` test, prints the matrix and sshd evidence, tears down.
#
# Usage: crates/demeteo-core/tests/conformance/ssh_cert/run-ssh-cert-spike.sh
# Backend: DEMETEO_CERT_BACKEND=docker|local (default: docker if the daemon is
# reachable, else a rootless local sshd on a loopback port). No network needed
# for `local`; `docker` needs to pull/apt-get.
# DEMETEO_CERT_SSHD_EXTRA=<directive> appends to the local sshd_config.
#
# Unverified: the `docker` backend has never been executed (the spike host had
# no daemon and no network), so `Dockerfile.cert` has no recorded image digest
# and no matrix run against a Linux sshd. Only `local` has produced results.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../../../.." && pwd)"
PORT="${DEMETEO_SSH_CERT_PORT:-2299}"
BACKEND="${DEMETEO_CERT_BACKEND:-auto}"
if [ "$BACKEND" = auto ]; then
  if docker info >/dev/null 2>&1; then BACKEND=docker; else BACKEND=local; fi
fi

WORK="$(mktemp -d "${TMPDIR:-/tmp}/demeteo-ssh-cert.XXXXXX")"
LOG="$WORK/sshd.log"
: >"$LOG"
PIDS=()
CONTAINER="demeteo-ssh-cert-$$"
cleanup() {
  rc=$?
  [ "$rc" -ne 0 ] && { echo "==> FAILED (rc=$rc); sshd log tail:" >&2; tail -40 "$LOG" >&2 || true; }
  for p in ${PIDS[@]+"${PIDS[@]}"}; do kill "$p" 2>/dev/null || true; done
  for a in "$WORK"/agent_*.pid; do [ -f "$a" ] && kill "$(cat "$a")" 2>/dev/null || true; done
  [ "$BACKEND" = docker ] && docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT

if [ "$BACKEND" = docker ]; then USER_NAME=demeteo; else USER_NAME="$(id -un)"; fi
PRINCIPAL=demeteo

echo "==> backend=$BACKEND user=$USER_NAME port=$PORT"
echo "==> client: $(ssh -V 2>&1)"
echo "==> libssh2 (vendored): $(grep -h 'define LIBSSH2_VERSION ' "$(ls -d ~/.cargo/registry/src/*/libssh2-sys-0.3.2 | head -1)/libssh2/include/libssh2.h" | awk '{print $3}' | tr -d '"')"

# ---- CAs and keys ---------------------------------------------------------
# Layout the Rust test reads: $WORK/<variant>/<type>{,.pub,-cert.pub}
ssh-keygen -q -t ed25519 -N '' -C ca -f "$WORK/ca"
ssh-keygen -q -t ed25519 -N '' -C badca -f "$WORK/badca_ca"
echo "$PRINCIPAL" >"$WORK/auth_principals"
mkdir -p "$WORK/good" "$WORK/badca" "$WORK/expired" "$WORK/ctl"

ssh-keygen -q -t ed25519 -N '' -C ed25519 -f "$WORK/good/ed25519"
ssh-keygen -q -t rsa -b 3072 -N '' -C rsa -f "$WORK/good/rsa"
ssh-keygen -q -t ecdsa -b 256 -N '' -C ecdsa -f "$WORK/good/ecdsa"
for t in ed25519 rsa ecdsa; do
  ssh-keygen -q -s "$WORK/ca" -I "cert-$t" -n "$PRINCIPAL" -V +5m "$WORK/good/$t.pub"
done
for v in badca expired; do cp -p "$WORK/good/ed25519" "$WORK/good/ed25519.pub" "$WORK/$v/"; done
ssh-keygen -q -s "$WORK/badca_ca" -I cert-badca -n "$PRINCIPAL" -V +5m "$WORK/badca/ed25519.pub"
ssh-keygen -q -s "$WORK/ca" -I cert-expired -n "$PRINCIPAL" -V -10m:-5m "$WORK/expired/ed25519.pub"
ssh-keygen -q -t ed25519 -N '' -C ctl -f "$WORK/ctl/ctl"
cp "$WORK/ctl/ctl.pub" "$WORK/ctl.pub"
echo "==> certificates:"
for f in "$WORK"/good/*-cert.pub "$WORK"/badca/*-cert.pub "$WORK"/expired/*-cert.pub; do
  ssh-keygen -L -f "$f" | sed -n '1,2p;/Valid:/p;/Signing CA/p;/Principals/,+1p' | sed "s|^|    |"
done

# ---- sshd -----------------------------------------------------------------
if [ "$BACKEND" = local ]; then
  SSHD="$(command -v sshd || echo /usr/sbin/sshd)"
  echo "==> server: $("$SSHD" -V 2>&1 | head -1)"
  ssh-keygen -q -t ed25519 -N '' -f "$WORK/host_key"
  cp "$WORK/ctl.pub" "$WORK/authorized_keys"; chmod 600 "$WORK/authorized_keys"
  cat >"$WORK/sshd_config" <<CFG
Port $PORT
ListenAddress 127.0.0.1
HostKey $WORK/host_key
PidFile $WORK/sshd.pid
LogLevel VERBOSE
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
PubkeyAuthentication yes
StrictModes no
TrustedUserCAKeys $WORK/ca.pub
AuthorizedPrincipalsFile $WORK/auth_principals
AuthorizedKeysFile $WORK/authorized_keys
PerSourcePenalties no
MaxAuthTries 20
CFG
  # Optional extra sshd directives, e.g. 'PubkeyAcceptedAlgorithms +ssh-rsa-cert-v01@openssh.com'.
  [ -n "${DEMETEO_CERT_SSHD_EXTRA:-}" ] && printf '%s\n' "$DEMETEO_CERT_SSHD_EXTRA" >>"$WORK/sshd_config"
  "$SSHD" -D -e -E "$LOG" -f "$WORK/sshd_config" &
  PIDS+=("$!")
else
  docker build -q --build-arg "SSH_USER=$USER_NAME" -t demeteo-ssh-cert:latest \
    -f "$HERE/../sshd/Dockerfile.cert" "$WORK" >/dev/null
  echo "==> image: $(docker image inspect demeteo-ssh-cert:latest --format '{{.Id}}')"
  docker run -d --name "$CONTAINER" -p "127.0.0.1:$PORT:22" demeteo-ssh-cert:latest >/dev/null
  docker exec "$CONTAINER" sshd -V 2>&1 | head -1 | sed 's/^/==> server: /' || true
  docker logs -f "$CONTAINER" >"$LOG" 2>&1 &
  PIDS+=("$!")
fi

for i in $(seq 1 30); do
  if (exec 3<>"/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then exec 3>&- 3<&-; break; fi
  [ "$i" -eq 30 ] && { echo "sshd did not come up" >&2; cat "$LOG" >&2; exit 1; }
  sleep 1
done

# ---- one isolated agent per key type, holding key + its cert ----------------
for t in ed25519 rsa ecdsa; do
  sock="$WORK/agent_$t.sock"
  eval "$(ssh-agent -a "$sock" -s)" >/dev/null
  echo "$SSH_AGENT_PID" >"$WORK/agent_$t.pid"
  SSH_AUTH_SOCK="$sock" ssh-add "$WORK/good/$t" 2>&1 | sed "s/^/    ssh-add[$t]: /"
  echo "    agent[$t] identities: $(SSH_AUTH_SOCK=$sock ssh-add -l | awk '{print $2" "$NF}' | tr '\n' ';')"
done

# ---- run ---------------------------------------------------------------------
export DEMETEO_SSH_CERT_HOST=127.0.0.1 DEMETEO_SSH_CERT_PORT="$PORT"
export DEMETEO_SSH_CERT_USER="$USER_NAME" DEMETEO_SSH_CERT_DIR="$WORK" DEMETEO_SSH_CERT_LOG="$LOG"
export DEMETEO_SSH_CERT_AGENT_ED25519="$WORK/agent_ed25519.sock"
export DEMETEO_SSH_CERT_AGENT_RSA="$WORK/agent_rsa.sock"
export DEMETEO_SSH_CERT_AGENT_ECDSA="$WORK/agent_ecdsa.sock"

cd "$REPO_ROOT"
cargo test -p demeteo-core --test ssh_cert -- --ignored --nocapture --test-threads=1 2>&1 \
  | grep -v '^\s*\(Compiling\|Finished\|Running\|Blocking\|Downloaded\|Updating\)'
