# Demeteo Hub — operator guide

This directory deploys the Hub as four Docker Compose services:

| Service | Role |
|---|---|
| `hub` | the axum server (`crates/demeteo-hub`), built from `deploy/hub/Dockerfile` |
| `postgres` | the Hub's relational state |
| `openbao` | custody: the Transit engine wraps attachment keys and the Hub's own secrets |
| `caddy` | TLS termination; the only service that publishes ports (80, 443) |

Why the Hub is shaped this way, and what it will and will not hold, is decided in
[docs/HUB.md](../../docs/HUB.md) (§9 for storage and custody). This guide is only the
operator procedure; it does not restate those decisions.

Every command below runs from `deploy/hub/`. Copy `.env.example` to `.env`, replace every
placeholder, and never commit `.env`.

Compose refuses to start without `POSTGRES_PASSWORD`, and it builds `DATABASE_URL` by pasting
that value verbatim into `postgres://user:password@…`. The password must therefore be URL-safe —
only letters, digits, `-`, `_`, `.` and `~` — or be percent-encoded. A `#`, `?`, `/` or `@` in it
breaks the URL and the Hub cannot connect. `openssl rand -hex 32` gives a long password that
needs no encoding.

## First run: initialise OpenBao

OpenBao starts uninitialised and sealed. Initialise it once:

```bash
docker compose up -d openbao
docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 openbao bao operator init
```

The command prints the unseal keys and the initial root token exactly once. Store them
offline (a password manager or printed copies in separate places), and never commit them,
paste them into a ticket, or put them in `.env`. Losing the unseal keys loses everything
OpenBao wraps, including every attachment key.

The root token is for the setup steps below and break-glass use only. The Hub never gets it.

## Unseal after every restart

OpenBao comes up sealed after **every** restart of the container or host, and stays sealed
until a threshold of unseal keys is entered:

```bash
docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 openbao bao operator unseal
```

Run it once per key until the threshold is met, then check with
`docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 openbao bao status`.

This is an operator step. Nothing in the stack automates it: there is no init or unseal
sidecar, and the Hub does not wait for it. With its token already configured, the Hub
survives a sealed OpenBao — `/healthz` stays 200, and the operations that need a key return a
sealed error until you unseal. In that case nothing crash-loops, and nothing needs restarting
once you have unsealed. A missing token is different; see [Start the stack](#start-the-stack).

## Enable Transit and create the Hub's key

With OpenBao unsealed, log in with the root token and enable the Transit engine:

```bash
docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=<root-token> openbao \
  bao secrets enable transit
docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=<root-token> openbao \
  bao write -f transit/keys/demeteo-hub
```

The mount (`transit`) and key name (`demeteo-hub`) are `DEMETEO_HUB_TRANSIT_MOUNT` and
`DEMETEO_HUB_TRANSIT_KEY`; change both sides together if you pick other names.

### A least-privilege policy and token

The Hub needs `update` on three paths of one key and nothing else: encrypt, decrypt, and a
plaintext data key. Write the policy through stdin, then mint a token from it:

```bash
docker compose exec -i -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=<root-token> openbao \
  bao policy write demeteo-hub - <<'EOF'
path "transit/encrypt/demeteo-hub"          { capabilities = ["update"] }
path "transit/decrypt/demeteo-hub"          { capabilities = ["update"] }
path "transit/datakey/plaintext/demeteo-hub" { capabilities = ["update"] }
EOF

docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=<root-token> openbao \
  bao token create -policy=demeteo-hub -period=720h
```

Put the resulting `client_token` in `.env` as `OPENBAO_TOKEN`. Do not use the root token as
`OPENBAO_TOKEN`, and do not add `-no-default-policy`: OpenBao's `default` policy is what lets
the token renew and look itself up.

### Renew the token inside every period

**The Hub does not renew its token.** A `-period=720h` token expires 720 hours after it was
minted or last renewed, and from then on every Transit call returns 403 — the Hub keeps
serving, but every operation that needs a key fails with a permission error rather than a
sealed one, and nothing restarts or alerts. Renew it with the Hub's own token, which resets
the clock to a full period:

```bash
docker compose exec -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN=<hub-token> openbao \
  bao token renew
```

Schedule that well inside the period rather than relying on memory — weekly is a comfortable
margin for 720 hours. A cron entry or systemd timer on the host can read the token from `.env`
so it never appears on a command line; `-e BAO_TOKEN` with no value passes the caller's
variable through:

```bash
# e.g. weekly from cron: 0 4 * * 1  /path/to/deploy/hub/renew-token.sh
cd "$(dirname "$0")" && set -a && . ./.env && set +a
BAO_TOKEN="$OPENBAO_TOKEN" docker compose exec -T \
  -e BAO_ADDR=http://127.0.0.1:8200 -e BAO_TOKEN openbao bao token renew >/dev/null
```

Check the time left with `bao token lookup` run the same way (its `ttl` field). A renewal
against a sealed OpenBao fails, so unseal first and let the next run catch up — or renew by
hand — if the host was down. Renewal inside the Hub process is later work; until it exists,
this schedule is what keeps the Hub's key operations working.

## Start the stack

Start everything only once `OPENBAO_TOKEN` is in `.env`:

```bash
docker compose up -d
```

The token is required at startup. If `OPENBAO_TOKEN` is missing or empty the Hub logs an
error naming it and exits, and under `restart: unless-stopped` it keeps restarting in a loop until the token is set *and* the
container is recreated — so an early `docker compose up -d` leaves a crash-looping `hub`.

After any later change to `.env`, recreate the Hub so it picks the change up:

```bash
docker compose up -d hub
```

`docker compose restart` does not re-read `.env`; it restarts the container with the
environment it was created with. Check the result with `docker compose ps`: `hub` should
show `Up`, not `Restarting`. If it is restarting, `docker compose logs hub` prints the error
it exited with.

## HTTPS, the RP ID and a stable domain

The Hub must be served over HTTPS on a domain you intend to keep. WebAuthn binds every
passkey to the relying-party identifier (the RP ID, `DEMETEO_HUB_RP_ID`), which is the
registrable domain the browser sees. Changing the domain, or the RP ID, invalidates every
enrolled passkey; there is no migration. Choose it once.

`localhost` is the only development exception, because browsers treat it as a secure context.
For anything else, plain HTTP will not work.

Caddy obtains and renews the certificate through ACME, which needs:

- DNS for `DEMETEO_HUB_DOMAIN` pointing at this host;
- ports 80 and 443 reachable from the public internet (80 for the challenge).

`DEMETEO_HUB_DOMAIN`, `DEMETEO_HUB_PUBLIC_URL` and `DEMETEO_HUB_RP_ID` must agree.

## Environment

| Variable | Read by | Meaning |
|---|---|---|
| `DATABASE_URL` | hub | Postgres connection URL. Compose builds it from the `POSTGRES_*` values |
| `DEMETEO_HUB_BIND` | hub | listen address inside the container |
| `DEMETEO_HUB_PUBLIC_URL` | hub | the externally visible `https://` URL |
| `DEMETEO_HUB_RP_ID` | hub | WebAuthn RP ID; see above |
| `DEMETEO_HUB_WEB_DIR` | hub | directory of static files to serve, default `/srv/hub-web`. Neither the image nor compose ships a bundle there yet, so every static request is 404 until `hub-web/` exists |
| `OPENBAO_ADDR` | hub | OpenBao address (compose network only) |
| `OPENBAO_TOKEN` | hub | the least-privilege Transit token |
| `OPENBAO_TOKEN_FILE` | hub | path to a file holding that token, as an alternative to `OPENBAO_TOKEN` |
| `DEMETEO_HUB_TRANSIT_MOUNT` | hub | Transit mount, default `transit` |
| `DEMETEO_HUB_TRANSIT_KEY` | hub | Transit key name |
| `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` | postgres, hub | database credentials; the password is required and must be URL-safe (see above) |
| `DEMETEO_HUB_DOMAIN`, `DEMETEO_HUB_UPSTREAM` | caddy | site name and the `hub` address to proxy to |
| `BAO_API_ADDR` | openbao | the address OpenBao advertises |
| `DEMETEO_HUB_TEST_DATABASE_URL` | tests only | Postgres URL for the ignored tests |

## Running the ignored tests

The tenancy and OpenBao round-trip tests need live services, so they are `#[ignore]`d and a
plain `cargo test` skips them. Bring up Postgres and an unsealed OpenBao with Transit and the
key set up as above, then:

```bash
export DEMETEO_HUB_TEST_DATABASE_URL=postgres://<user>:<password>@<host>:<port>/<db>
export OPENBAO_ADDR=<address reachable from the host>
export OPENBAO_TOKEN=<the least-privilege token>
export DEMETEO_HUB_TRANSIT_KEY=demeteo-hub
cargo test -p demeteo-hub -- --ignored
```

The least-privilege token is enough. The missing-key test asserts only that OpenBao refuses a
key the Hub was not given; with the policy above that refusal is the ACL layer's 403, which
the test accepts alongside an unknown-key error from a broader token.

The compose file publishes neither Postgres nor OpenBao, so from the host you need a
temporary port mapping or a `docker compose run` of your own. A later ticket adds a CI job
that runs these; until then they are run by hand.

## What this deployment does not protect

The bootstrap secrets sit in plaintext on the host: `POSTGRES_PASSWORD` and `OPENBAO_TOKEN`
are in `.env` and in the container environment, and anyone who can read either can use them.
Transit protects the attachment keys and the Hub's own secrets *from Postgres*; it does not
protect OpenBao's access token from whoever controls the host. `OPENBAO_TOKEN_FILE` keeps the
token out of the process environment but not off the disk.

OpenBao's storage is the `file` backend on its own named volume (`openbao-data`). Back that
volume up, together with the offline unseal keys: one without the other cannot be restored.
