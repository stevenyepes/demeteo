#!/usr/bin/env bash
#
# Set up and run a development Demeteo Hub on this machine.
#
# Postgres and OpenBao run in Docker; the Hub runs on the host through
# `cargo run`, serving a freshly built `hub-web/dist` on http://localhost:<port>
# — the procedure of "A local passkey sign-in" in deploy/hub/README.md, plus the
# OpenBao setup that pairing a desktop needs.
#
# ## Isolation from a real deployment
#
# Everything runs under its own Compose project, `demeteo-hub-dev`, so its
# volumes never collide with a `demeteo-hub` stack started from deploy/hub/.
# Its state lives in deploy/hub/.dev/ (gitignored, and outside the Docker build
# context), never in deploy/hub/.env.
#
# ## Unseal material on disk
#
# The README forbids keeping unseal keys or the root token next to a real
# deployment. This script keeps both in deploy/hub/.dev/openbao-init.json so it
# can unseal on every start — acceptable only because this OpenBao wraps nothing
# but a dev Hub's keys. Do not point a deployment at this state.
#
# ## Usage
#
#   scripts/dev-hub.sh            # set up whatever is missing, then run the Hub
#   scripts/dev-hub.sh --no-web   # skip `npm run build:hub-web`
#   scripts/dev-hub.sh stop       # stop Postgres and OpenBao, keep their data
#   scripts/dev-hub.sh status     # containers, seal state, the token's ttl
#   scripts/dev-hub.sh reset      # delete the dev volumes and state, after a prompt
#
# DEMETEO_HUB_DEV_PORT picks the Hub's port (default 8080). The browser must
# open http://localhost:<port>/ — the Hub refuses any other origin, 127.0.0.1
# included. A paired desktop pins the Hub's URL and signing key, so changing
# the port or running `reset` means pairing it again.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEPLOY_DIR="$REPO_ROOT/deploy/hub"
STATE_DIR="$DEPLOY_DIR/.dev"
ENV_FILE="$STATE_DIR/env"
INIT_FILE="$STATE_DIR/openbao-init.json"
PORT="${DEMETEO_HUB_DEV_PORT:-8080}"

die() { echo "dev-hub: $*" >&2; exit 1; }
say() { echo "dev-hub: $*"; }

for tool in docker jq openssl cargo npm; do
  command -v "$tool" >/dev/null || die "$tool is required"
done

compose() {
  docker compose -p demeteo-hub-dev --project-directory "$DEPLOY_DIR" \
    --env-file "$ENV_FILE" \
    -f "$DEPLOY_DIR/compose.yaml" -f "$DEPLOY_DIR/compose.test.yaml" "$@"
}

# Bare `-e BAO_TOKEN` forwards the caller's value, so no token is ever on a
# command line. Callers set BAO_TOKEN for the one call that needs it.
bao() {
  compose exec -T -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN openbao bao "$@"
}

ensure_state() {
  mkdir -p "$STATE_DIR"
  chmod 700 "$STATE_DIR"
  if [[ ! -f "$ENV_FILE" ]]; then
    umask 077
    printf 'POSTGRES_USER=demeteo\nPOSTGRES_PASSWORD=%s\nPOSTGRES_DB=demeteo_hub\n' \
      "$(openssl rand -hex 32)" >"$ENV_FILE"
    say "created $ENV_FILE"
  fi
  # shellcheck disable=SC1090
  set -a; . "$ENV_FILE"; set +a
}

save_env_var() {
  local name="$1" value="$2"
  sed -i "/^$name=/d" "$ENV_FILE"
  printf '%s=%s\n' "$name" "$value" >>"$ENV_FILE"
  export "$name=$value"
}

# `up --wait` returns once the container runs, before OpenBao's listener
# accepts. `bao status` exits 0 unsealed, 2 sealed, 1 unreachable.
wait_for_openbao() {
  local rc=1
  for _ in $(seq 1 30); do
    bao status >/dev/null 2>&1 && rc=0 || rc=$?
    [[ "$rc" -ne 1 ]] && return 0
    sleep 1
  done
  compose logs openbao >&2
  die "OpenBao was not reachable within 30s"
}

openbao_field() {
  { bao status -format=json 2>/dev/null || true; } | jq -r ".$1"
}

init_openbao() {
  [[ "$(openbao_field initialized)" == "true" ]] && return 0
  [[ -f "$INIT_FILE" ]] && die "OpenBao is uninitialised but $INIT_FILE exists; run 'reset'"
  say "initialising OpenBao"
  (umask 077; bao operator init -format=json >"$INIT_FILE")
}

unseal_openbao() {
  [[ "$(openbao_field sealed)" == "false" ]] && return 0
  [[ -f "$INIT_FILE" ]] || die "OpenBao is sealed and $INIT_FILE is missing; run 'reset'"
  say "unsealing OpenBao"
  local key
  for key in $(jq -r '.unseal_keys_b64[]' "$INIT_FILE"); do
    bao operator unseal "$key" >/dev/null
    [[ "$(openbao_field sealed)" == "false" ]] && return 0
  done
  die "OpenBao is still sealed after every saved unseal key"
}

# The key names, mount and policy are the README's, which the Hub's defaults
# match, so the Hub needs no DEMETEO_HUB_TRANSIT_* variables.
setup_transit() {
  local BAO_TOKEN
  BAO_TOKEN="$(jq -r '.root_token' "$INIT_FILE")"
  export BAO_TOKEN
  if ! bao secrets list -format=json | jq -e 'has("transit/")' >/dev/null; then
    say "enabling Transit and creating the Hub's keys"
    bao secrets enable transit >/dev/null
    bao write -f transit/keys/demeteo-hub >/dev/null
    bao write -f transit/keys/demeteo-hub-signing type=ed25519 >/dev/null
    bao policy write demeteo-hub - >/dev/null <<'EOF'
path "transit/encrypt/demeteo-hub"          { capabilities = ["update"] }
path "transit/decrypt/demeteo-hub"          { capabilities = ["update"] }
path "transit/datakey/plaintext/demeteo-hub" { capabilities = ["update"] }
path "transit/keys/demeteo-hub-signing"      { capabilities = ["read"] }
path "transit/sign/demeteo-hub-signing"      { capabilities = ["update"] }
EOF
  fi
  unset BAO_TOKEN
}

# The Hub never renews its own token (README, "Renew the token inside every
# period"), so renew it on every start and mint a new one once it has lapsed.
ensure_hub_token() {
  if [[ -n "${OPENBAO_TOKEN:-}" ]] && BAO_TOKEN="$OPENBAO_TOKEN" bao token renew >/dev/null 2>&1; then
    return 0
  fi
  say "minting the Hub's OpenBao token"
  local token
  token="$(BAO_TOKEN="$(jq -r '.root_token' "$INIT_FILE")" \
    bao token create -policy=demeteo-hub -period=720h -format=json | jq -er '.auth.client_token')"
  save_env_var OPENBAO_TOKEN "$token"
}

cmd_up() {
  local build_web=1
  for arg in "$@"; do
    case "$arg" in
      --no-web) build_web=0 ;;
      *) die "unknown option: $arg" ;;
    esac
  done

  ensure_state
  compose up -d --wait postgres openbao
  wait_for_openbao
  init_openbao
  unseal_openbao
  setup_transit
  ensure_hub_token

  cd "$REPO_ROOT"
  if [[ "$build_web" -eq 1 ]]; then
    npm run build:hub-web
  fi

  say "Hub on http://localhost:$PORT/ (Ctrl-C to stop; 'scripts/dev-hub.sh stop' for the containers)"
  DATABASE_URL="postgres://$POSTGRES_USER:$POSTGRES_PASSWORD@127.0.0.1:5432/$POSTGRES_DB" \
  DEMETEO_HUB_BIND="127.0.0.1:$PORT" \
  DEMETEO_HUB_PUBLIC_URL="http://localhost:$PORT" \
  DEMETEO_HUB_RP_ID=localhost \
  DEMETEO_HUB_WEB_DIR=hub-web/dist \
  OPENBAO_ADDR=http://127.0.0.1:8200 \
  OPENBAO_TOKEN="$OPENBAO_TOKEN" \
  RUST_LOG="${RUST_LOG:-info}" \
    exec cargo run -p demeteo-hub
}

cmd_stop() {
  ensure_state
  compose stop postgres openbao
}

cmd_status() {
  ensure_state
  compose ps
  if [[ -n "$(compose ps -q --status running openbao)" ]]; then
    bao status || true
    if [[ -n "${OPENBAO_TOKEN:-}" ]]; then
      BAO_TOKEN="$OPENBAO_TOKEN" bao token lookup -format=json | jq -r '"hub token ttl: \(.data.ttl)s"' || true
    fi
  fi
}

cmd_reset() {
  [[ -d "$STATE_DIR" ]] || { say "nothing to reset"; return 0; }
  ensure_state
  read -r -p "Delete the dev Hub's database, OpenBao keys and $STATE_DIR? Paired desktops must pair again. [y/N] " answer
  [[ "$answer" == [yY] ]] || die "aborted"
  compose down -v
  rm -rf "$STATE_DIR"
  say "reset done"
}

case "${1:-up}" in
  up) shift || true; cmd_up "$@" ;;
  --*) cmd_up "$@" ;;
  stop) cmd_stop ;;
  status) cmd_status ;;
  reset) cmd_reset ;;
  *) die "unknown command: $1 (up | stop | status | reset)" ;;
esac
