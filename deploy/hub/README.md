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
For anything else, plain HTTP will not work. [A local passkey sign-in](#a-local-passkey-sign-in)
is the procedure that uses it.

Caddy obtains and renews the certificate through ACME, which needs:

- DNS for `DEMETEO_HUB_DOMAIN` pointing at this host;
- ports 80 and 443 reachable from the public internet (80 for the challenge).

`DEMETEO_HUB_DOMAIN`, `DEMETEO_HUB_PUBLIC_URL` and `DEMETEO_HUB_RP_ID` must agree.

## A local passkey sign-in

No automated test drives a real browser or a real authenticator: the browser tests double
`navigator.credentials`, and the server tests sign with a software authenticator. Registering a
passkey, signing out and signing back in with a real one is a manual check, made against the
Hub itself on `http://localhost`.

Sign-in needs Postgres and nothing from OpenBao. Start Postgres through the test override, which
publishes it on `127.0.0.1:5432`:

```bash
docker compose -f compose.yaml -f compose.test.yaml up -d --wait postgres
```

Then, from the repository root and not from `deploy/hub/`, build the browser bundle and run the
Hub on the host:

```bash
npm run build:hub-web
DATABASE_URL=postgres://<user>:<password>@127.0.0.1:5432/<db> \
DEMETEO_HUB_BIND=127.0.0.1:8080 \
DEMETEO_HUB_PUBLIC_URL=http://localhost:8080 \
DEMETEO_HUB_RP_ID=localhost \
DEMETEO_HUB_WEB_DIR=hub-web/dist \
OPENBAO_ADDR=http://127.0.0.1:8200 \
OPENBAO_TOKEN=unused \
RUST_LOG=info \
cargo run -p demeteo-hub
```

| Value | Why it is this |
|---|---|
| `DEMETEO_HUB_WEB_DIR` | `hub-web/dist` is where `npm run build:hub-web` writes the bundle; the Hub serves it and the `/v1` routes from one origin |
| `DEMETEO_HUB_PUBLIC_URL` | `http://localhost:` followed by the port in `DEMETEO_HUB_BIND`. The Hub compares the origin a browser reports with this one byte for byte, so a different port, or `127.0.0.1` in place of `localhost`, is refused |
| `DEMETEO_HUB_RP_ID` | `localhost`, the only RP ID a browser accepts over plain HTTP |
| `OPENBAO_ADDR`, `OPENBAO_TOKEN` | the Hub refuses to start without them, but no sign-in route calls the key service. Any non-empty values do: it logs `key service is not usable yet; serving anyway` and serves |
| `RUST_LOG` | the Hub logs errors only by default, so without it a healthy start prints nothing |

`cargo run` builds the Hub on the host, where it links the system OpenSSL (the note on
`webauthn-rs` in `crates/demeteo-hub/Cargo.toml`), so the OpenSSL development package has to be
installed.

Open `http://localhost:8080/` — `localhost`, not `127.0.0.1` — and:

1. follow **Sign up**, name the passkey and create it. The navigation appears only once a
   session exists;
2. **Sign out**, then **Sign in with a passkey**. No name is asked for: the authenticator offers
   the passkey it holds for `localhost`;
3. optionally, under **Passkeys**, add a second passkey from a second authenticator. It is listed
   as pending and cannot sign in until **Endorse** on its row is answered with the first one.
   **Revoke** asks a different trusted passkey, and the last trusted one cannot be revoked.

### Why not the Vite dev server

`npm run dev:hub-web` serves the page from an origin of its own, and the Hub accepts exactly one.
A state-changing request whose `Origin` is not the origin of `DEMETEO_HUB_PUBLIC_URL` is answered
`403 origin_mismatch`, and a registration or an assertion whose `clientDataJSON` names another
origin fails the WebAuthn origin check. `vite.hub-web.config.ts` configures no proxy either, so a
`/v1` request from that page reaches Vite and never the Hub. The session cookie is a third fence:
it is `SameSite=Strict`, so a page on another site never has it sent.

None of these is relaxed for development, and neither is `Secure`: the cookie carries it on
`http://localhost` exactly as it does behind Caddy, for the reason on `set_cookie` in
`crates/demeteo-hub/src/auth/session.rs`. A browser that treats `http://localhost` as a secure
context stores it anyway; Chromium and Firefox are expected to, and Safari is not verified. In
one that does not, the passkey is created and the session does not stick. The answer there is
HTTPS, never a weaker cookie.

Stop the Hub with Ctrl-C and the database with `docker compose stop postgres`. The account stays
in the `postgres-data` volume, and the passkey stays on the authenticator under the RP ID
`localhost` until it is deleted there.

## Environment

| Variable | Read by | Meaning |
|---|---|---|
| `DATABASE_URL` | hub | Postgres connection URL. Compose builds it from the `POSTGRES_*` values |
| `DEMETEO_HUB_BIND` | hub | listen address inside the container |
| `DEMETEO_HUB_PUBLIC_URL` | hub | the externally visible `https://` URL. Its origin is the one every passkey ceremony and every state-changing request is checked against |
| `DEMETEO_HUB_RP_ID` | hub | WebAuthn RP ID; see above |
| `DEMETEO_HUB_WEB_DIR` | hub | directory of static files to serve, default `/srv/hub-web`. Neither the image nor compose ships a bundle there, so in this deployment every static request is 404 and the sign-in page is not served; [A local passkey sign-in](#a-local-passkey-sign-in) points it at a built `hub-web/dist` |
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

The tenancy, passkey-store and OpenBao round-trip tests need live services, so they are
`#[ignore]`d and a plain `cargo test` skips them. `compose.yaml` publishes neither Postgres nor
OpenBao, so start the two through the test override, which publishes both on `127.0.0.1` only:

```bash
docker compose -f compose.yaml -f compose.test.yaml up -d --wait postgres openbao
```

Adding the override to services that are already up recreates their containers, so OpenBao
comes back sealed. Initialise or unseal it, and set up Transit, the key and the token as above,
then:

```bash
export DEMETEO_HUB_TEST_DATABASE_URL=postgres://<user>:<password>@127.0.0.1:5432/<db>
export OPENBAO_ADDR=http://127.0.0.1:8200
export OPENBAO_TOKEN=<the least-privilege token>
export DEMETEO_HUB_TRANSIT_KEY=demeteo-hub
cargo test -p demeteo-hub -- --ignored
```

`--ignored` runs only the live tests; CI passes `--include-ignored` to run them with the
rest. Run a later `docker compose up` from a different shell, or `unset OPENBAO_ADDR` first: an
exported value outranks `.env`, and the Hub container would be told to find OpenBao on its own
loopback.

The least-privilege token is enough. The missing-key test asserts only that OpenBao refuses a
key the Hub was not given; with the policy above that refusal is the ACL layer's 403, which
the test accepts alongside an unknown-key error from a broader token.

### In CI

The `hub` job in `.github/workflows/pr-checks.yml` does all of this on every pull request and
on every push to `master`: it starts `postgres` and `openbao` through the override, runs the
init, unseal, Transit, policy and token steps of this guide in order, then
`cargo test -p demeteo-hub-protocol` and `cargo test -p demeteo-hub -- --include-ignored`, and
finally builds the image from `deploy/hub/Dockerfile` and discards it — nothing is pushed.

It is deliberately not part of `scripts/checks.sh`: `npm run checks` has to run on a machine
without Docker, so a green local run says nothing about these live tests.

The job reads the unseal keys and the root token out of `bao operator init` in a script. That
is acceptable only because its OpenBao exists for one job and wraps nothing worth keeping.
Never do the same for a deployment; [First run](#first-run-initialise-openbao) is the rule
there.

## No per-client limit, one replica

Nothing in this stack limits how many requests one client may make. The `caddy` service is the
stock image, which carries no rate-limit module, and its `Caddyfile` is a bare `reverse_proxy`.
The Hub does not count requests per address either. This is a known gap in the Hub, not a
setting you got wrong, and no value in `.env` closes it.

What the Hub does bound is how many passkey ceremonies it holds between a `…/start` request and
its `…/finish`. The constants are in `crates/demeteo-hub/src/auth/ceremonies.rs` and
`crates/demeteo-hub/src/auth/challenge.rs`:

| Constant | Value | What it bounds |
|---|---|---|
| `CHALLENGE_TTL_SECS` | 300 | seconds a started ceremony is held before it expires |
| `MAX_LIVE_CEREMONIES` | 4096 | registrations and sign-ins in flight, counted together over every caller |
| `MAX_LIVE_PER_HOLDER` | 8 | ceremonies in flight started by the sessions of one signed-in passkey |
| `MAX_LIVE_SESSION_BOUND` | 16,384 | ceremonies in flight started by any session, over every passkey |

Each of those budgets is shared, so without a per-client rate limit one client can use up what
was meant for everyone:

- **Signed out.** A registration and a sign-in need no account to start, and both count toward
  `MAX_LIVE_CEREMONIES` for `CHALLENGE_TTL_SECS`. A script that sustains roughly 14 starts a
  second keeps that budget full, and `POST /v1/session/start` and `POST /v1/register/start` then
  answer `503 too_many_ceremonies` to everyone. For as long as it lasts, a user who is not
  already signed in cannot sign in — including to revoke a compromised passkey.
- **Signed in.** Adding, endorsing and revoking a passkey are started by a session and held
  under the passkey that signed it in, `MAX_LIVE_PER_HOLDER` each, so a flood of sign-in starts
  does not refuse them. All of them together stop at `MAX_LIVE_SESSION_BOUND`, though, and
  registration is open: someone who first registers about two thousand passkeys of their own
  can fill that bound from their sessions. Every tenant's endorsement and revocation starts are
  then answered `503 too_many_ceremonies` as well.

**Do not expose the Hub to the public internet without a per-client request limit in front of
it** — a Caddy build that includes a rate-limit module, or another proxy or a firewall that has
one. Until then, keep the Hub on a private network or a VPN. The certificate procedure under
[HTTPS, the RP ID and a stable domain](#https-the-rp-id-and-a-stable-domain) assumes ports 80
and 443 are reachable from the public internet, so it does not fit a private Hub as written;
this guide does not yet describe another way to obtain the certificate.

### Run exactly one `hub`

Ceremonies in flight are held in the `hub` process's memory and nowhere else. A finish that
reaches a different replica than the one that served its start finds nothing there, so do not
scale `hub` beyond one, and do not put a second stack behind the same domain.

### What a restart of `hub` loses

A restart of `hub` — the `docker compose up -d hub` that follows a change to `.env` included —
drops every ceremony in flight. A user who was part-way through one is answered
`ceremony_expired` and starts it again. Sessions and passkeys are in Postgres and survive.

## What this deployment does not protect

The bootstrap secrets sit in plaintext on the host: `POSTGRES_PASSWORD` and `OPENBAO_TOKEN`
are in `.env` and in the container environment, and anyone who can read either can use them.
Transit protects the attachment keys and the Hub's own secrets *from Postgres*; it does not
protect OpenBao's access token from whoever controls the host. `OPENBAO_TOKEN_FILE` keeps the
token out of the process environment but not off the disk.

OpenBao's storage is the `file` backend on its own named volume (`openbao-data`). Back that
volume up, together with the offline unseal keys: one without the other cannot be restored.
