# Demeteo Hub — Design and Decisions

> **Scope:** how a browser control plane reaches many Demeteo instances — who
> trusts whom, how an instance pairs, what crosses the wire, what the Hub may ask
> an instance to do, and what never leaves an instance. **Built:** in part. The
> design is decided (2026-10-02); §2 says per part what is implemented.
> [`hub-design/README.md`](hub-design/README.md) owns the mockup,
> [`ARCHITECTURE.md`](ARCHITECTURE.md) owns the hexagon the desktop-side client
> sits in, [`MCP_INTEGRATION.md`](MCP_INTEGRATION.md) owns the other
> external surface into a desktop — a different surface, with a different trust
> argument (§8) — and [AGENTS.md §2](../AGENTS.md) owns the invariants this
> document must not weaken.
>
> The wire field names in §5 and §6 are **illustrative** unless a section says a
> name is fixed by decision. The decisions fix the behaviour;
> `crates/demeteo-hub-protocol` fixes the spelling, and where a name here differs
> from the crate's, the crate is the contract.

---

## 1. Summary

Decisions [62–69](DECISIONS.md#1-the-locked-decisions) are the short form of what
follows. Each row here has a section below that states the rejected alternative in
full.

| # | Decision | Rejected | Why |
|---|----------|----------|-----|
| 62 | A browser control plane (the Hub) is in scope; reverses [`OPEN_QUESTIONS.md` §17](OPEN_QUESTIONS.md#17-other-captured-items) | keeping a web companion out of scope | one view over a fleet, and a gate decided away from the machine, are needs the desktop cannot meet |
| 63 | Single tenant: one person, who also operates the Hub | hosted multi-tenant (the 2026-10-02 answer); teams | a Hub that mints git tokens (71) would put every tenant's repositories behind one breach; the passkey endorsement chain has no notion of membership |
| 64 | Desktops and runners pair and each hold one outbound `wss`; the Hub dials neither | unpaired runners relayed by a desktop (the 2026-10-02 answer); the Hub dialing runners over SSH; the Hub storing PATs or long-lived SSH keys | a run must start while the desktop is off; NAT; a runner's pairing secrets are sealed by `systemd-creds` — see [decision 64](DECISIONS.md#64--hub-instances-detail) |
| 65 | The Hub builds future runner `RunSpec`s from desktop-reported snapshots with local paths stripped | Hub-side authoring of Projects and Workflows; Workflows read from repo files | a second editor plus drift; a new concept |
| 66 | A Gate may be decided from the Hub by a WebAuthn assertion the desktop verifies against pinned passkeys | a session plus a step-up for some gates; read-only gates; manual per-desktop passkey acceptance | a compromised Hub could approve ordinary gates; see §8 |
| 67 | Scopes `read` / `spend` / `gates`, toggled and enforced on the instance; local per-run and rolling-24-hour ceilings | a `logs` scope; MCP's `configure`; toggle-only scopes | transcripts and diffs never leave an instance; a Hub-supplied cap is a cap the Hub can raise — see §7 |
| 68 | Postgres plus OpenBao (Transit) as Docker Compose: `hub`, `postgres`, `openbao`, `caddy` | HashiCorp Vault; Cosmian/Eviden KMS | BSL is not open source; source-available licensing — see §9 |
| 69 | `demeteo-hub`, `demeteo-hub-protocol` and `hub-web/` | the Hub linking `demeteo-core` in v1 | it would ship `libssh2`, git plumbing and `rusqlite` in a hosted container for code v1 never calls |
| 71 | The Hub mints a short-lived git token per runner run, and stores none: from a GitHub App key held in OpenBao, or from a GitLab bot user's OAuth refresh token | a runner storing its own git credential; the Hub storing PATs; SSH certificate authorities | nothing standing on an idle runner; one place to revoke; acceptable only single-tenant — see [decision 71](DECISIONS.md#71--runner-git-credentials-detail) |

---

## 2. Status of the parts

| Part | State |
|---|---|
| `crates/demeteo-hub` (axum server) | **partly built** — Postgres storage and migrations, the `KeyService` port with an OpenBao Transit adapter, `GET /healthz`, static serving of the browser bundle, and passkey auth under `/v1`: first-passkey registration, sign-in, sessions, and the endorsement and revocation of passkeys (§8); pairing (§4): `GET /authorize`, the three consent routes under `/v1/pairing`, and `POST /token` with the `authorization_code` and `refresh_token` grants; a runner's device-code pairing: `POST /device_authorization`, the four approval routes under `/v1/pairing/device` with the Hub verifying the passkey assertion, the device-code grant of `POST /token`, and each instance's `kind`; and the instance socket at `GET /socket` (§5): admission by a DPoP-bound access token, the registry of live connections and each instance's presence, the run mirror with its cursor acks, the latest snapshot, and the queue of signed requests with the outcome recorded for each; and the session API under `/v1` (§6.4): the fleet, runs and request-status reads, and the routes that queue `start_feature`, `cancel_feature`, `set_step_assignment` and, after a gate challenge, `gate_decision`; and attachments (§6.3): the upload under `/v1`, a dispatch that names them, the fetch-once at `GET /attachments/{attachment_id}` admitted like the socket, and the TTL sweep. It depends on `crates/demeteo-hub-protocol`. A paired desktop dials the socket, and the requests queued for it are delivered, verified and answered there, gate decisions included (§6.1 *On the desktop*, §8). **Not built:** a route that queues `passkey_endorsement` or `passkey_revocation` for an instance (§10), and unpairing and revoking an instance |
| `crates/demeteo-hub-protocol` (wire types) | **built** — the socket's messages, the signed envelope and its six kinds, snapshots (Features and harness kinds included, decision 70), and the canonical encoder of §6.1 and §8. The Hub and `demeteo-core` both compile against it |
| `hub-web/` (second Vite entry) | **sign-in, passkey management, the two pairing approval pages and the Fleet and Instance reads built, the rest an app shell** — the SignUp, SignIn and Passkeys views call the Hub's `/v1` routes, ApprovePairing at `#/pair/<segment>` calls the consent routes (§4), ApproveDevice at `#/device` and `#/device/<user_code>` looks a runner's code up and approves it with a passkey, and every route but sign-up and sign-in is behind the session. Fleet reads `GET /v1/fleet` (§6.4) and leads with a Needs-you lane of pending gates, Instance reads `GET /v1/instances/{instance_id}`, both show whether an instance is a desktop or a runner, and AddInstance explains pairing from the desktop and `demeteo-runner hub pair`; Runs, Run and Dispatch are placeholders with no data wiring yet, so no view uploads an attachment. `npm run build:hub-web` writes a static bundle to `hub-web/dist`; `demeteo-hub` serves the directory its configuration names |
| The Docker Compose bundle (`hub`, `postgres`, `openbao`, `caddy`) | **built** — `deploy/hub`, with its own operator README |
| Desktop side: device key, pairing, fingerprint | **built**, in `demeteo-core` under `ExecutionMode::Router` only (§9 *Code layout*) — the P-256 device key, the loopback callback, the `/authorize` and `/token` exchange, and the `hub_pair`, `hub_status` and `hub_disconnect` commands, whose typed wrappers are in `src/lib/hub.ts`. `hub_status` answers the fingerprint in the format of §4. The refresh token is in the keyring and the access token in process memory only; `HubPairing::refresh_access` rotates them, one `refresh_token` grant at a time. Unpairing is local-only (§4 *Revocation*), and stops the socket first. **For a runner** (§4 *Pairing a runner by device code*), `demeteo-core` also has the device-code client (`HubPairing::begin_device_pairing`) and the sealed store (`SealedSecretStore`, migration `V68__hub_sealed_secrets.sql`), and composes both in `ExecutionMode::LocalOnly` when the caller hands it a store (`build_core_context_with_hub_store`). `demeteo-runner serve` holds that pairing and starts its socket; `demeteo-runner hub pair`, `status`, `policy` and `disconnect` call the `hub_*` methods of its control socket (`crates/demeteo-runner/src/rpc/hub.rs`), which a desktop has no Settings surface for yet; and `demeteo-runner hub seal`, which both installers run, seals the store key and writes the unit drop-in that loads it |
| Desktop side: socket client | **built**, under `ExecutionMode::Router` only (§9 *Code layout*) — it starts when the app starts paired, or when `hub_pair` succeeds, and stops on `hub_disconnect`. It sends `Hello`, then the allowlisted events of the desktop's own `run_events` above the Hub's cursor, refreshes the access token when it holds no live one, on a `4001` close and on an upgrade `401`, and reconnects with backoff unless a verdict stops it. Each request it receives goes to the request handler on a task of its own, and the handler takes one start at a time, across reconnects and re-pairs alike, while cancels and assignments do not wait behind one, and the handler's `RequestResult`, when there is one, is sent on the open connection — the next one, if the connection that carried the request has closed, but not after an app restart or a `hub_pair`, which drop it (§6.1 *On the desktop*). Events from **local runs only** reach the Hub (§5); a snapshot after the first `CursorAck` and after each Project or Workflow write (§5 *Snapshots*). The `hub-socket` feature no longer implies `keyring`, so a runner build can carry the socket; a runner's socket has no request handler bound, so its requests stay queued |
| Desktop side: Settings › Hub tab | **built** — `src/components/settings/HubTab.tsx`. Outside `ExecutionMode::Router` it shows only the local toggles, ceilings, passkeys and activity log. It pairs and disconnects; shows the fingerprint, this machine as the Hub sees it (`hub_machine`), and the socket's state, sync, Hello → first-ack latency and why it stopped (`hub_connection`); sets the toggles and ceilings of §7 through `hub_policy_set`; lists the trusted passkeys as pinned or endorsed (`hub_trusted_passkeys`); and shows the activity log (`hub_request_log`). It polls the connection and the log every 5 s while open. `spend` and `gates` still default to off, so every request that needs one is refused until the user turns it on |
| Desktop side: passkey pin set, request verification, local ceilings | **built** — the request-signing key, RP ID and passkey public keys are stored by migration `V64__hub_pairing.sql`, and each passkey's signature counter by `V66__hub_passkey_sign_count.sql`. Every request is verified against the pinned request-signing key and judged against the local toggles and ceilings, which migration `V65__hub_requests.sql` stores with the consumed ids and the activity log (§6.1 *On the desktop*, §7). The WebAuthn assertion a `gate_decision` or a `passkey_revocation` carries is verified on the desktop against the pinned set before the request has any effect: a gate is then decided by the desktop's own `gate_decide`, and a key is removed in one transaction with the signer's counter (§8). Every `passkey_endorsement` is consumed and refused `unknown_kind` without its assertion being read (§11 item 20). A gate of a run a `demeteo-runner` owns is not decided this way yet: it is refused `runner_owned` (§10) |
| An instance-wide read over `run_events` | **built** — `RunEventsPort::list_after` pages every Feature in offset order (§5); the socket client's backfill and live send read through it |
| Runner dispatch from the Hub | **decided, not built** — a runner pairs by device code and holds its socket (§4 and the rows above), but answers no request yet, and the Hub mints no git token (decisions 64 and 71); see §10 |

The design mockup in [`hub-design/`](hub-design/README.md) exists. It is a
picture of the Hub, not a part of it, and where it disagrees with this document
this document wins; the README lists where.

---

## 3. Trust model

**An instance trusts its own keyring and its own toggles. It trusts the Hub for
delivery and nothing else.** A compromised Hub is a real case, and the design is
judged by what that attacker cannot do.

### Never leaves an instance

| Item | Why it stays |
|---|---|
| Provider API keys | a `ProviderInstance` key lives in the OS keyring only ([AGENTS.md §2](../AGENTS.md)) |
| Git tokens | the same rule; an instance clones and pushes with its own |
| Harness sign-ins | `~/.claude`, `~/.codex` and their kind are never read, copied or mutated |
| Local paths | snapshots strip them (§5); the Hub names a Project by an id the instance reported, never by a path |
| Transcripts | there is no scope that sends them — §7 |
| Diffs | the same |
| File contents | the same; an event is an allowlisted projection, not a file |

> **Do not add a `logs` scope.** The design opened with one — a toggle that sent
> transcripts and diffs to the Hub when an instance turned it on. It was dropped
> on 2026-10-02. A per-instance toggle is a fine control for *spending*, where the
> worst outcome is a bounded bill. It is the wrong control for content: once a
> transcript is on a hosted multi-tenant server, the toggle cannot take it back,
> and the Hub's custody story (§9) is only true while there is none. A feature
> that seems to need a diff on the Hub needs a summary the instance writes, or a
> trip to the desktop — not a scope.

### What the Hub does hold

| Held | Notes |
|---|---|
| Tenant, passkey **public** keys, instance records, device-key thumbprints | no private key of any kind |
| Refresh-token hashes, access-token metadata | the plaintext leaves once, in `/token`'s response (the same rule as [MCP §5](MCP_INTEGRATION.md#5-authorization)) |
| Events and snapshots an instance chose to send | status-level; §5 |
| Queued requests | until delivery or expiry |
| **Attachments**, briefly | user-authored content the browser sent: a file the user attached to a Feature. Encrypted at rest, deleted after the instance fetches it — §6.3 |
| Feature titles and harness kinds | the other user-authored content on the Hub: the title of each of an instance's newest Features, scrubbed and cut to 200 characters, in its snapshot, and the kinds of harness it registered. Sent only with `read` on, replaced by the next snapshot ([decision 70](DECISIONS.md#1-the-locked-decisions), §5). A Feature's description, prompts and PR text have no field |
| Its own server secrets | in OpenBao, or encrypted under Transit — §9 |
| The GitHub App private key, **decided, not built** | imported into OpenBao Transit and used there, never in the Hub process; it mints a one-repository token per runner run, which is never stored ([decision 71](DECISIONS.md#71--runner-git-credentials-detail)) |

### What a compromised Hub can and cannot do

| Can | Cannot |
|---|---|
| read what an instance reported: project and workflow names, step status | read a key, a transcript, a diff or a path — none was sent; nor a cost, which no protocol message reports (§5) |
| withhold, delay or reorder requests | forge a request: the envelope is signed and the instance verifies it against the key it pinned (§6) |
| replay a request | replay one that works: ids are single-use and expire (§6) |
| queue a `start_feature` for an instance that has `spend` on | declare a cap above the instance's per-run ceiling, or one that takes the 24-hour committed total above its rolling ceiling (§7). The ceilings bound declared caps, not what a run spends — §7 *The per-run ceiling bounds the declared cap, not what the run spends* |
| relay a gate decision a human made | **make** one with no ceremony: no token approves, and the assertion is verified on the desktop (§8). That holds per ceremony: a substituted ceremony can spend one (the next row) |
| substitute what a passkey ceremony signs. It serves the hub-web bundle that runs every ceremony (§2), so a ceremony the user takes for a sign-in or one gate approval can be fed another challenge, and the authenticator shows only the RP. That ceremony can decide one gate the user did not mean, or revoke one of the user's other pinned keys (§11 item 20) | add a key to an instance's pinned set, with a ceremony or without: the desktop refuses every `passkey_endorsement` as `unknown_kind` without reading its assertion (§11 item 20); nor revoke the last key (§8) |
| rewrite the free-text `feedback` a relayed gate decision carries, which no assertion binds. The desktop discards it, so a rewrite changes nothing on the instance (§8) | change the decision, its gate or its request: the challenge binds all three (§8) |
| — | turn a scope on, or raise a ceiling |

> **A stolen device key and refresh token** let an attacker speak as the instance:
> report false events and collect queued requests, attachments included. That is
> why the refresh token rotates with reuse detection (§4), and why the device key
> is created on the instance and kept in the keyring: it never exists anywhere
> that can be copied from a hosted service.

---

## 4. Pairing

Pairing makes one **desktop** an instance of one tenant. A `demeteo-runner` host
pairs by device code approved with a passkey
([decision 64](DECISIONS.md#64--hub-instances-detail)), whose Hub half is built
and is the last subsection below; until then what follows is the desktop's. It reuses the shape of the OAuth listener in
[MCP §5](MCP_INTEGRATION.md#5-authorization) — loopback, PKCE `S256`, a consent the
human signs — and differs from it on purpose.

> **Do not "align" this with MCP's token.** MCP's is a 30-day bearer with no
> refresh because the listener it protects is on the same machine. The Hub is
> remote and hosted, so the access token is **15 minutes**, **bound to a key**, and
> the refresh token rotates. A copy of an MCP token is a local problem; a copy of a
> Hub token is a tenant's.

The Hub's half is built, in `crates/demeteo-hub/src/pairing/`, and so is the
approval page in hub-web. The desktop's half is built in `demeteo-core`, short of
refreshing its tokens (§2). What follows states the Hub's side as that code does
it, and is the contract the desktop client is built to.

<!-- EXAMPLE: new -->

```
instance            device key: create, keep in the OS keyring           (once)
instance ─browser─▶ GET  /authorize   PKCE S256, redirect http://127.0.0.1:<port>/…,
                                      dpop_jkt = thumbprint of the device key
Hub ─303─────────▶ /#/pair/<segment>  the approval page, behind the passkey sign-in
browser ─────────▶ POST /v1/pairing/start     fingerprint, instance labels, scopes asked for
browser ─────────▶ POST /v1/pairing/approve   the scopes the user left checked
browser ─────────▶ http://127.0.0.1:<port>/…?code=…&state=…
instance ────────▶ POST /token        code + code_verifier + redirect_uri, DPoP proof
Hub ─────────────▶ access token (15 min, bound to the thumbprint)
                   refresh token (rotating, reuse-detected)
                   pairing response: instance id, request-signing key, RP ID, passkey keys
```

| Property | Value |
|---|---|
| Device key | created **on the instance**, private half in the OS keyring only; the Hub is sent the public half. ES256 (P-256) is the only algorithm the Hub accepts, and is a carried default (§11). The Hub stores the public half as the four-member JWK it rebuilds from the decoded point, never the client's JSON. The desktop's key is therefore P-256, signing ES256 proofs, generated with `aws-lc-rs` in `crates/demeteo-core/src/adapters/hub/device_key.rs` |
| Fingerprint | a short rendering of the key's RFC 7638 thumbprint, shown **on both** the desktop and the Hub's approval page so the user can compare them. The format is fixed — below |
| Redirect | `http://127.0.0.1:<port>/…` on an **ephemeral port** the instance opens for the one callback (RFC 8252); never a custom scheme, never `localhost`. The rule is below. The desktop binds `127.0.0.1:0`, so the OS chooses the port, and serves one random path per pairing (`crates/demeteo-core/src/adapters/hub/loopback.rs`) |
| PKCE | `S256`, mandatory (RFC 7636) |
| Authorization code | single-use and good for 60 seconds, bound to the device key by `dpop_jkt` (RFC 9449), so a code intercepted on the loopback cannot be redeemed with another key. A redemption that fails burns the code |
| Sign-in | a **passkey**; the Hub has no password |
| Consent | the user approves the device-key fingerprint and the scopes (§7), and may approve fewer than were asked for — never more, and never none |
| `/token` | a DPoP proof on every request (RFC 9449), each proof accepted once |
| Access token | **15 minutes**, bound to the device-key thumbprint. It is opaque — 32 random bytes — so the binding is the row the Hub holds for its hash, not a claim inside it. It is presented with a fresh DPoP proof on every use |
| Refresh token | **rotating**: each use returns a new pair and retires the old refresh token. It lives 30 days, a carried default (§11) |
| Reuse detection | a retired refresh token presented again **revokes the whole pairing** — a stolen token and the real one cannot both survive. There is no grace window |

### `GET /authorize`

The route reads no cookie and holds no state, so it is reachable signed out. Every
parameter is required. One the Hub does not name is ignored — there is no
`client_id` (§11) — and one sent twice refuses the request.

| Parameter | Rule |
|---|---|
| `response_type` | exactly `code` |
| `code_challenge_method` | exactly `S256` |
| `code_challenge` | unpadded base64url of 32 bytes |
| `redirect_uri` | the loopback rule below |
| `dpop_jkt` | the device key's RFC 7638 thumbprint: unpadded base64url of 32 bytes |
| `scope` | one or more of `read`, `spend`, `gates` (§7), separated by single spaces, none repeated |
| `state` | 1–128 characters of `A-Z a-z 0-9 - . _ ~` |
| `instance_name` | 1–65 characters after trimming |
| `os`, `arch`, `app_version` | 1–32 characters each after trimming |

`instance_name`, `os`, `arch` and `app_version` are **self-asserted**: they are
what the caller says about itself, they are shown on the approval page as claimed
by the machine, and they prove nothing. A label holding a control character, a
zero-width character, a bidirectional control or a byte-order mark is refused,
because the name a user reads would not be the characters that were sent.

**The loopback rule.** `redirect_uri` is accepted only if it is, byte for byte,
`http://127.0.0.1:`, a port of one to five digits with no leading zero and a value
of 1–65535, and a path that begins with `/` and holds only `/` and
`A-Z a-z 0-9 - . _ ~`, in at most 255 bytes. No other scheme or letter case, no
`localhost`, no other address or other spelling of this one, no userinfo, query,
fragment, percent-encoding, backslash or whitespace.

> **Do not parse this URI, and do not widen the rule.** It is a comparison of
> bytes on purpose. The authorization code leaves in this redirect, and a URL
> parser normalises before it answers — `127.1`, `2130706433`, a host after an
> `@` — so a browser may resolve what the parser approved to somewhere else. The
> rustdoc of `is_loopback_redirect` in
> [`authorize.rs`](../crates/demeteo-hub/src/pairing/authorize.rs) has the
> reasoning, and why the desktop OAuth listener's wider rule does not apply.

**A valid request** is answered `303 See Other` with `Location: /#/pair/<segment>`
and `Cache-Control: no-store`. `<segment>` is the validated request as base64url,
opaque to hub-web. It carries no secret and no signature and needs neither: the
consent routes decode it and run the same validation again.

**A refusal is `400 {"error":"invalid_request"}`, whatever was wrong, and is
never a redirect to the client.** RFC 6749 has an authorization server redirect
an error to the `redirect_uri` it registered for the client. The Hub registers no
client, so the only address it has is the one in the request being refused.

### The consent routes

`/#/pair/<segment>` is hub-web's approval page. Like every hub-web route but
sign-up and sign-in it is behind the session, so a signed-out user signs in with a
passkey first and lands back on it. The page calls three routes, each of which
needs the session cookie, a matching `Origin` and the CSRF header:

| Route | Request body | `200` body |
|---|---|---|
| `POST /v1/pairing/start` | `{"request": "<segment>"}` | `{"pairing_id", "fingerprint", "instance_name", "os", "arch", "app_version", "scopes": ["read", …], "expires_at": <Unix seconds>}` |
| `POST /v1/pairing/approve` | `{"pairing_id", "scopes": […]}` | `{"redirect": "<redirect_uri>?code=<code>&state=<state>"}` |
| `POST /v1/pairing/deny` | `{"pairing_id"}` | `{"redirect": "<redirect_uri>?error=access_denied&state=<state>"}` |

- `start` parks the request for the session's user for 300 seconds. `fingerprint`
  is rendered by the Hub; hub-web shows it and never derives it.
- `approve` and `deny` spend the `pairing_id`, which only the user it was parked
  for can answer. `approve` takes a non-empty subset of the scopes that were asked
  for, with none repeated, and mints the authorization code.
- The browser then leaves for `redirect`. hub-web follows it only if it is a
  loopback URL of the shape above.
- Refusals: `400 invalid_request` (a body or segment that does not validate),
  `400 pairing_expired` (a `pairing_id` that is unknown, expired, already
  answered or another user's), `400 invalid_scope`, `503 too_many_pairings`, and
  the session's own `401` and `403`.

A pending consent, a pending code and the proofs already seen are held in the
Hub's memory, bounded (§11), and are lost when it restarts.

### The fingerprint

**`SHA256:`, then the first 16 characters of the base64url thumbprint, in four
groups of four separated by one space.** The thumbprint
`0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I` is shown as:

```
SHA256:0ZcO CORZ NYy- DWpq
```

That is 96 bits of the thumbprint. **The desktop must render the identical
string from the same thumbprint**: the two screens are compared by eye, and a
difference in prefix, grouping or length reads as a different key. The format is
a carried default (§11) and is not the Hub's alone to change — `fingerprint` in
[`jwk.rs`](../crates/demeteo-hub/src/pairing/jwk.rs) writes it, and
`the_thumbprint_and_fingerprint_match_a_published_key` in
`src-tauri/tests/hub_pairing.rs` pins the example above, which is the key of
RFC 9449's own example proof. The desktop's copy is `fingerprint` in
`crates/demeteo-core/src/domain/hub_pairing/jwk.rs`, and
`the_desktop_fingerprint_is_the_one_the_hub_shows_for_its_thumbprint` in
`src-tauri/tests/hub_desktop_pairing.rs` holds the two to the same output.

The thumbprint is the SHA-256 of exactly
`{"crv":"P-256","kty":"EC","x":"<x>","y":"<y>"}`, with `x` and `y` re-encoded from
the decoded coordinates, as unpadded base64url.

### `POST /token`

The body is `application/x-www-form-urlencoded`, and the request carries exactly
one `DPoP` header. The route sits outside `/v1` and its `Origin` and CSRF guard,
because a desktop is not a browser; a request is held to its proof instead. Every
answer, a refusal included, carries `Cache-Control: no-store`.

**The proof** is a compact JWS of at most 4096 bytes:

| Part | Rule |
|---|---|
| Header | `typ` is `dpop+jwt`; `alg` is `ES256` and nothing else; `jwk` is the device key's public half — `kty` `EC`, `crv` `P-256`, 32-byte `x` and `y`, and no `d` |
| `htm` | `POST` |
| `htu` | the Hub's configured origin followed by `/token`, compared as a whole string. It is never built from a request's `Host` |
| `iat` | within 60 seconds of the Hub's clock, either side |
| `jti` | 1–128 bytes. The Hub remembers it with the key's thumbprint for as long as the proof could still be accepted, and refuses a second use |
| Signature | the 64-byte raw `r‖s` over `<header>.<payload>` |

**`grant_type=authorization_code`** takes `code`, `code_verifier` (43–128
characters of `A-Z a-z 0-9 - . _ ~`) and `redirect_uri`. The Hub verifies the
proof, then takes the code — so a redemption that fails from here on has burned it
— and requires that the code is not expired, that `redirect_uri` is the string
that was approved, that the SHA-256 of `code_verifier` is the `code_challenge`,
and that the proof was made by the key `dpop_jkt` named. It then reads the
request-signing public key (§9) and, in one transaction, writes the instance, its
device key and its first token pair and reads the tenant's passkeys.

**`grant_type=refresh_token`** takes `refresh_token`. The Hub verifies the proof,
takes its turn on the device key the token is bound to, so that two refreshes of
one key never run together, and requires, in this order, that the token exists,
is a refresh token, is bound to the proof's key, has not been retired, and has
not expired. A rotation retires the presented token and issues a new pair in the
same family, bound to the same key; the access token issued with the old one
stays good until its own expiry. A retired token presented again with the right
key is reuse: every live token of the family is retired, the newest pair
included, and that is committed although the answer is a refusal. A used refresh
token is remembered until its own expiry and no longer: a rotation deletes the
key's expired tokens as it commits, and one presented after that is an unknown
token, refused with the same `invalid_grant`.

**The response.** Both grants answer `200` with JSON. A refresh carries the first
four members and nothing else:

| Member | Value | In a refresh |
|---|---|---|
| `access_token` | unpadded base64url of 32 random bytes | yes |
| `token_type` | `DPoP` | yes |
| `expires_in` | `900` | yes |
| `refresh_token` | unpadded base64url of 32 random bytes | yes |
| `scope` | what the user approved, space-separated | no |
| `instance_id` | a UUID **the Hub mints** — below | no |
| `request_signing_key` | unpadded base64url of the 32-byte Ed25519 public key | no |
| `rp_id` | the Hub's configured WebAuthn RP ID | no |
| `passkeys` | `[{"credential_id", "public_key"}]`: the tenant's **trusted** passkeys, each as unpadded base64url of the raw credential id and of the COSE_Key bytes | no |

<!-- EXAMPLE: new -->

```
{
  "access_token":        "…",
  "token_type":          "DPoP",
  "expires_in":          900,
  "refresh_token":       "…",
  "scope":               "read gates",
  "instance_id":         "0f4c2f0e-7d0b-4c59-9d5e-6f1c1a2b3c4d",
  "request_signing_key": "…",
  "rp_id":               "hub.example",
  "passkeys":            [{ "credential_id": "…", "public_key": "…" }]
}
```

The plaintext of a token exists in this one answer. Postgres holds the SHA-256 of
each token's 32 bytes, and an authorization code is held in memory by its hash.

**Refusals**

| Status | `error` | When |
|---|---|---|
| 400 | `invalid_request` | a form that cannot be read, or a missing field |
| 400 | `unsupported_grant_type` | any other `grant_type` |
| 400 | `invalid_dpop_proof` | no `DPoP` header or more than one, a proof that fails any rule above, a replayed `jti` |
| 400 | `invalid_grant` | every refusal of a code or of a refresh token — unknown, expired, used, wrong verifier, wrong redirect, wrong key, reused. One answer, so a caller cannot probe which check failed |
| 503 | `key_service_unavailable` | the request-signing key could not be read: OpenBao is sealed or unreachable, or the key does not exist yet |
| 503 | `too_many_requests` | the Hub is remembering as many proofs as it will hold |
| 500 | `internal` | a store failure, or a request-signing key the Hub is misconfigured for — a Transit key that is not `ed25519` among them |

### The pairing response

The `/token` response of the pairing grant carries three things the instance pins.
A refresh returns none of them.

| Field | Member | Pinned as | Used for |
|---|---|---|---|
| The Hub's **request-signing public key** | `request_signing_key` | the key every envelope is verified against (§6) | rejecting a request the Hub did not sign |
| The Hub's **WebAuthn RP ID** | `rp_id` | the RP ID every assertion must be made for (§8) | rejecting an assertion made for another site |
| The user's **passkey public keys** | `passkeys` | the initial pinned set (§8) | verifying a gate decision |

The passkeys are the ones trusted when the code is redeemed, read in the
transaction that writes the pairing, not the ones the tenant had when the user
approved.

The pinned set is **trust on first pair**: the instance has no way to check these
three values at pairing except that they arrived over the TLS connection the user
chose and that the user approved the pairing on a screen they signed into. After
pairing, nothing the Hub says changes the passkey set on its own authority (§8).

### The instance id

**The Hub mints `instance_id`**, when a device key is first paired, and returns it
in the pairing response. It is the `instance_id` every envelope names (§6.1) and
every challenge binds (§8). The desktop does not send an id of its own at
pairing: what it sends is the four self-asserted labels, which the Hub stores as
the instance's name, OS, architecture and version.

### Pairing the same device key again

**Pairing a device key the tenant has already paired replaces the old pairing.**
The instance keeps its `instance_id`; its labels and granted scopes become this
pairing's; every live token bound to the key is retired in the transaction that
writes the new pair, so the earlier pairing's tokens stop working when it commits.
The key is looked up among that tenant's keys alone: a thumbprint another tenant
holds is a first pairing here.

**Rejected: refusing a key that is already paired.** Reuse detection retires a
whole family, and a refresh whose answer was lost looks the same as a stolen one,
so an honest desktop can find every token it holds retired. **Pairing again is the
recovery**, and the only one: with a refusal that desktop would have no way back.

### Revocation

Two things retire a pairing's tokens today: reuse detection, and pairing the same
device key again. Both retire the access tokens with the refresh tokens.

**Unpairing from the desktop is built and local-only.** `hub_disconnect` deletes
the device key and the refresh token from the keyring and the pins from SQLite,
and tells the Hub nothing, because the Hub has no route to tell. The Hub keeps
listing the instance, and the refresh token stays valid there, held by nobody,
until it expires 30 days after it was issued. The next pairing proves a fresh
device key, so it is a first pairing, not a replacement.

A revocation that reaches the Hub — from the desktop's unpair or from the Hub
itself — is designed and **not built** (§10): it retires the refresh token family,
and takes effect by the next refresh at the latest, so at most one access-token
lifetime (15 minutes) after the Hub-side revoke.

### Pairing a runner by device code

A `demeteo-runner` host has no browser and often no route back from one, so it
pairs by the device authorization grant of RFC 8628, held to the same device key,
DPoP binding and pairing response as the desktop's grant
([decision 64](DECISIONS.md#64--hub-instances-detail)). Everything not named here
is as above: ES256 device key, the fingerprint format, rotating refresh tokens
with reuse detection, the Hub minting `instance_id`, a second pairing of the same
key replacing the first.

<!-- EXAMPLE: new -->

```
runner ──────────▶ POST /device_authorization   labels + scope, DPoP proof of the device key
Hub ─────────────▶ device_code, user_code, verification_uri, expires_in 600, interval 5
runner prints      the user_code, the verification URI and its own fingerprint
human ──browser──▶ /#/device → enters the user_code, compares the fingerprint,
                   approves with a passkey assertion made for this pairing
runner ──────────▶ POST /token   grant_type=urn:ietf:params:oauth:grant-type:device_code,
                                 device_code, DPoP proof — every `interval` seconds
Hub ─────────────▶ authorization_pending … then the pairing response of the
                   authorization-code grant, member for member
```

**`POST /device_authorization`** sits outside `/v1` beside `/token`, with no
`Origin` or CSRF guard, `application/x-www-form-urlencoded`, and every answer
`Cache-Control: no-store`. It carries exactly one `DPoP` header, held to the rules
of `/token` except that `htu` is the configured origin followed by
`/device_authorization`. The device key is the proof's `jwk`, so there is no
`dpop_jkt` parameter: the key is proven, not named. The form is `scope`,
`instance_name`, `os`, `arch` and `app_version`, under the rules of `GET /authorize`.

| `200` member | Value |
|---|---|
| `device_code` | unpadded base64url of 32 random bytes; held by its SHA-256, in memory |
| `user_code` | eight characters of `BCDFGHJKLMNPQRSTVWXZ`, shown as two groups of four joined by `-` |
| `verification_uri` | the configured origin followed by `/#/device` |
| `verification_uri_complete` | `verification_uri`, then `/` and the `user_code` |
| `expires_in` | `600` |
| `interval` | `5` |

Refusals: `400 invalid_request` and `400 invalid_dpop_proof` as on `/token`, and
`503 too_many_pairings` at the bound.

**The device-code grant** is `POST /token` with
`grant_type=urn:ietf:params:oauth:grant-type:device_code` and `device_code`. The
proof must be made by the key that started the pairing. Its answers:

| Status | `error` | When |
|---|---|---|
| 200 | — | approved: the pairing response, every member of the authorization-code grant's. The `device_code` is spent |
| 400 | `authorization_pending` | not answered yet |
| 400 | `slow_down` | polled sooner than `interval` after the last poll; the runner adds 5 seconds to its interval, as RFC 8628 §3.5 says |
| 400 | `access_denied` | the human denied it. The `device_code` is spent |
| 400 | `expired_token` | ten minutes passed unanswered |
| 400 | `invalid_grant` | unknown or spent, or a proof by another key |
| 400 | `invalid_dpop_proof` | as on `/token` |

**Approval is a passkey assertion, not the session.** A desktop's consent is
answered by whoever holds the session, because the code it mints can only be
redeemed through the loopback of the machine that asked. A device code has no such
anchor: anyone can start one and hand its `user_code` to the human, which is the
risk decision 64 accepts and stands the passkey and the fingerprint against. So
hub-web's `#/device` page looks the code up, shows the claimed labels and the
Hub-rendered fingerprint, and the approve route takes a WebAuthn assertion over a
challenge the Hub issued for that one pending pairing — the ceremony
`gate_decision` uses (§8). Denying needs only the session. A lookup that misses
counts against a bound the session's tenant holds, so a `user_code` cannot be
guessed through it.

**The approval routes.** Each needs the session cookie, a matching `Origin` and
the CSRF header, as the consent routes do. `user_code` is read in any letter case,
with or without its `-`.

| Route | Request body | `200` body |
|---|---|---|
| `POST /v1/pairing/device/lookup` | `{"user_code"}` | `{"user_code", "fingerprint", "instance_name", "os", "arch", "app_version", "scopes": [...], "expires_at": <Unix seconds>}` |
| `POST /v1/pairing/device/challenge` | `{"user_code", "scopes": [...]}` | `{"ceremony_id", "options": {"publicKey": ...}}` |
| `POST /v1/pairing/device/approve` | `{"ceremony_id", "assertion": {"credential_id", "authenticator_data", "client_data_json", "signature"}}` | `{"answer": "approved"}` |
| `POST /v1/pairing/device/deny` | `{"user_code"}` | `{"answer": "denied"}` |

- The challenge is the SHA-256 of `device_approval_challenge` in
  [`canonical.rs`](../crates/demeteo-hub-protocol/src/canonical.rs): its own
  label, `demeteo-hub/v1/device`, then the device key's thumbprint, the scopes
  left checked, a fresh request id and an expiry. It is held for the session's
  passkey with the pairing it names, and `options` allows any of the tenant's
  trusted passkeys. `scopes` only narrows what the runner asked for, under the
  rule of the consent routes.
- Unlike a `gate_decision`'s, this assertion is verified **by the Hub**, against
  the tenant's trusted passkey it names, user verification required, before
  anything is recorded: no instance exists yet to verify it. The ceremony is
  spent by the attempt; a refused assertion leaves the pairing pending.
- The first answer wins, from whichever tenant gives it.
- Refusals: `400 invalid_request` (a body that does not validate, or text that
  cannot be a code), `400 pairing_expired` (no pending pairing has that code:
  never issued, expired, or answered), `400 invalid_scope`, `400
  ceremony_expired`, `403 assertion_invalid`, `429 too_many_lookups` (below),
  `503 too_many_ceremonies`, and the session's own `401` and `403`.
- **The miss bound.** Every route that takes a `user_code` looks it up, and a
  lookup that finds no pending pairing is a miss. A tenant with ten misses in its
  current ten-minute window is answered `429 too_many_lookups` for every lookup,
  the right code included, until the window ends. It is counted per tenant, so
  its sessions share one bound. Text that cannot be a code is not counted.

**An instance has a kind.** A pairing by this grant is a `runner` instance, and one
by the authorization-code grant a `desktop`. Pairing the same key again takes the
kind of the grant that paired it last. `instances.kind` holds it (migration
`0009_instance_kind.sql`), and `GET /v1/fleet` and `GET /v1/instances/{instance_id}`
answer it as `kind`.

**On the runner** the device key and refresh token are stored in its SQLite,
encrypted under a key that `systemd-creds` sealed and its unit loads with
`LoadCredentialEncrypted=` (decision 64). `demeteo-runner hub seal` creates that
key and a unit drop-in, and the installers run it. Pairing runs inside
`demeteo-runner serve`, which owns the secret store and the socket: `hub pair`,
`hub status`, `hub policy` and `hub disconnect` are clients of its control socket,
and a desktop's machine settings reach the same RPC methods over SSH. A `serve`
without the sealed key answers that it cannot pair, and why.

---

## 5. The socket

**One outbound `wss` per instance.** The instance opens it; the Hub never dials a
desktop (decision 64). Events travel up, requests travel down, over the same
connection. The access token and a DPoP proof are presented on the upgrade.

<!-- EXAMPLE: new -->

```
instance → Hub   hello      install id, version, OS, arch, granted scopes
Hub → instance   ack        cursor the Hub has durably stored
instance → Hub   events     run events with offset > cursor, in offset order
Hub → instance   ack        the new cursor
instance → Hub   snapshot   Project and Workflow state, local paths stripped
Hub → instance   request    one signed envelope (§6); queued ones first
instance → Hub   result     request id; accepted | refused, and why
```

| Frame | Protocol type | Direction | Carries |
|---|---|---|---|
| `hello` | `Hello` | instance → Hub | install id, app version, OS, arch, **granted scopes** — what the instance's toggles allow *now*, not what pairing approved |
| `ack` | `CursorAck` | Hub → instance | the highest event offset the Hub has stored durably |
| `events` | `RunEvent` | instance → Hub | **one** run event per frame, with its Feature and offset — not a batch |
| `snapshot` | `Snapshot` | instance → Hub | the instance's full current Project and Workflow state in one frame, replacing the last |
| `request` | `SignedRequest` | Hub → instance | one signed envelope |
| `result` | `RequestResult` | instance → Hub | the outcome of one request id: `accepted`, or `refused` with a reason — `expired` is one of the reasons, not a third outcome |

The names in the first column, and in the sketch above it, are this document's
shorthand. **None of them is on the wire**: what the wire carries is in *Frames*
below.

### The upgrade

The socket is `GET /socket`, outside `/v1` like `/token` and for its reason: a
desktop is not a browser and sends no `Origin`. The request carries two things
besides the WebSocket upgrade headers.

| Header | Rule |
|---|---|
| `Authorization` | `DPoP <access token>`: the scheme in any case, exactly one space, and the token's text as `/token` issued it |
| `DPoP` | exactly one, a proof as §4 specifies it for `/token`, with the three differences below |

| Proof claim | Rule on the socket |
|---|---|
| `htm` | `GET` |
| `htu` | the Hub's configured origin followed by `/socket`, in its **`https`** spelling. Never `wss`, although that is the scheme the client dials, and never built from a request's `Host`. `htu` is compared as a whole string, so there has to be one spelling, and it is the one a client already builds for `/token` |
| `ath` | **required**: unpadded base64url of the SHA-256 of the access token's ASCII text, exactly as it is sent in `Authorization` (RFC 9449 §4.2). `/token` presents no access token and ignores the claim; the socket refuses a proof without it, or with another token's |

The Hub verifies the proof, remembers its `jti`, checks `ath`, and only then
looks the token up: it must be an access token, bound to the key that made the
proof, not retired and not expired, and that key must still belong to an
instance. **The tenant and the instance a socket writes for come from that
token's row and that key, and from nothing the instance says afterwards.**

A proof is spent by being presented, whatever becomes of the upgrade it came
with. A client that is refused makes a new proof, with a new `jti`, for its next
attempt.

**Refusals.** Every one is answered before the upgrade, as plain HTTP, never as a
`101` followed by a close:

| Status | `error` | When |
|---|---|---|
| 401 | `invalid_token` | every credential failure: no `Authorization`, another scheme, no `DPoP` header or more than one, a proof that fails any rule, a replayed `jti`, a missing or wrong `ath`, a token that is unknown, a refresh token, bound to another key, retired or expired, a key that belongs to no instance. **One answer**, with `WWW-Authenticate: DPoP`, so a captured request cannot be used to learn which part of it was wrong |
| 400 | `invalid_request` | the credentials passed and the request is not a WebSocket upgrade. Asked only of a request that was admitted, so one without credentials is `401` whatever else it lacks, and it is asked before the connection is registered: such a request displaces no session. Its proof is spent all the same |
| 503 | `too_many_requests` | the Hub is remembering as many proofs as it will hold (§4). The token was not judged; retry, do not refresh |
| 503 | `too_many_connections` | as many instances are connected to this `hub` process as it will hold, and this one is not among them. No session is closed to make room |
| 500 | `internal` | a store failure |

### Frames

**A frame is a JSON text frame holding one protocol struct, and nothing around
it.** There is no envelope and no `type` member: `crates/demeteo-hub-protocol`
defines none, and the Hub adds none.

- **Inbound** is whichever of `Hello`, `RunEvent`, `Snapshot` and
  `RequestResult` the text parses as, and **exactly one must**. All four refuse
  unknown fields and no two share their required members, which is what makes
  that unambiguous. A frame that parses as none of them, or as more than one,
  closes the socket.
- **Outbound** is a `CursorAck` or a `SignedRequest`. They refuse unknown fields
  and share no required member either, so an instance can tell them apart the
  same way.
- A **binary** frame closes the socket, as does text that is not JSON, a frame
  with a member its type does not have, and a frame whose `v` is not the
  protocol version. Nothing of a refused frame is stored.

**Rejected for now: a tagged frame.** It is the cleaner shape, and it is a change
to the protocol crate that both ends share, not to how the Hub reads it. If it is
taken, it is taken there.

**Order.** The first frame must be `Hello`. Anything else first is a violation,
and so is a second `Hello`: a change of scopes is reported by reconnecting.
After it the instance may send events, snapshots and results in any order.

**One connection per instance.** A second connection of the same instance takes
the first one's place, and the first is closed with `4002`. An instance that
reconnects has usually lost a socket the Hub has not yet noticed is dead, so the
newcomer is taken for the live one.

**Liveness.** The Hub sends a WebSocket ping every 30 seconds. A connection on
which **no frame of any kind** arrives for 90 seconds — a pong counts, and so
does an event — is dropped, with no close frame, and the instance is marked
absent. Both numbers are carried defaults (§11). The same deadline covers a
connection that was admitted and never sent its `Hello`, and a peer that stops
reading is dropped on it as one that stops sending is: a write of the Hub's
still pending 90 seconds after it began ends the connection.

**Close codes.** What the Hub closes a socket with, and what the instance does
about it:

| Code | When | The instance |
|---|---|---|
| `1001` | the Hub is shutting down | reconnects |
| `1008` | a protocol violation: a frame before `Hello`, a second `Hello`, a binary or unparseable frame, a version mismatch, an event offset above 2⁶³−1, a U+0000 in any string of a frame, a `feature_id` over 256 bytes (`MAX_FEATURE_ID_BYTES`) | the same frame would be refused again |
| `1011` | the Hub could not do what a frame it accepted asked — a store call failed. An event it was storing was not acked. A frame the store can never take is not this: `frame::parse` refuses it, and it closes `1008` | reconnects and resumes from its last ack; sending the same thing may succeed |
| `4001` | the access token the socket was admitted with has expired | **refreshes and reconnects** |
| `4002` | displaced by a newer connection of the same instance | **does not reconnect in a loop**: it would displace its successor, which would reconnect in its turn |

The session ends at the expiry of the token it was *admitted* with — one
reconnect per access-token lifetime — and the instance is marked absent before
the close is sent. A `4001` on a token the desktop had just refreshed, which its
own clock still gives more than `PREMATURE_EXPIRY_MARGIN_SECS` to, is not that
expiry: the desktop backs off before refreshing, longer each time, rather than
rotate its refresh token as fast as the Hub closes. That, and closing on a
version mismatch, are defaults a human may still overrule (§11).

### Events and the cursor

Events come from the desktop's append-only `run_events` table
(`crates/demeteo-core/migrations/V22__run_events.sql`). Its `id` is
`INTEGER PRIMARY KEY AUTOINCREMENT`, so the **offset is table-global**: unique and
monotonic across every Feature on that desktop, not restarted per Feature.

**Only local runs are reported.** The desktop sends the rows of its own table and
nothing else (`crates/demeteo-core/src/adapters/hub/socket/session.rs`). A
detached run owned by `demeteo-runner` writes the runner's own `run_events`
(`crates/demeteo-runner/src/notify_bridge.rs`), whose offsets are a separate
sequence one instance cursor cannot cover, so a detached Feature does not reach
the Hub yet. Mirroring the runner's rows into the desktop's table would give
them new offsets, and against the `ExecutionPort` parity invariant
([AGENTS.md §2](../AGENTS.md)) that is a design change waiting on a decision,
not a fix.

**The cursor is per instance, not per Feature.** One high-water mark over the
table covers every Feature, which is what the Instance artboard's single offset
shows. A Feature's own history is still addressable because every event carries
its Feature. The existing read, `run_events_since(feature_id, from_offset)`
(`crates/demeteo-core/src/application/agent_surface/reads.rs`), is per Feature; the
instance-wide read is `RunEventsPort::list_after(from_offset, limit)`, served by the
primary key.

**Delivery is at-least-once and the Hub makes it idempotent.** An event is keyed
`(instance, feature, offset)`; the Hub drops a key it already holds. A dropped
connection costs a resend of whatever the last `ack` did not cover, never a gap
and never a duplicate row. The instance resumes from the Hub's `ack`, not from its
own memory of what it sent.

**When the Hub acks.** It sends one `CursorAck` after `Hello`, carrying the
cursor it has stored for the instance (`0` when it has stored nothing), and one
after **every** `RunEvent`. An ack is never sent before the write it reports has
committed: the event's row and the cursor are written in one transaction, and
the ack follows it. A re-sent event whose key the Hub already holds is dropped
**and still acked**, with the cursor where it stands. An event the Hub could not
store gets no ack: the socket closes with `1011`.

**"Highest contiguous offset" does not mean `n, n + 1`.** The cursor is the
highest offset `o` such that every event *received* from this instance with an
offset at or below `o` is durably stored. Offsets are sparse by design: the
sequence is table-global across Features, and an event whose kind is off the
allowlist below is dropped on the instance and never sent. A cursor that waited
for `o + 1` would stall at the first gap. The Hub processes one frame to
completion before it reads the next and ends the session on a failed write, so
the definition comes to the highest offset stored: after `3`, `7`, `20` the
cursor is `20`. It never moves backwards — an event re-sent from below the
cursor is stored if it was missing and leaves the cursor where it is.

**What a mirror row holds** is the parsed `RunEvent`, serialised again — never
the frame's text. **Nothing is stored for costs or gate summaries**, because the
protocol types carry neither: an event is a kind from a closed list and three
optional ids, and no kind reports a cost. Which gate summary text may cross at
all is open (§11).

**`ack` is not a delete.** `run_events` has no GC today
([`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md)), and an `ack` does not create one:
it moves the cursor and nothing else. If a GC ever lands, a cursor older than the
oldest retained event is a gap the Hub must rebuild from snapshots; how it is
signalled is open (§11).

**An event is an allowlisted projection.** The protocol crate lists the event
kinds and fields that cross, and the **instance** drops anything not on the list
before sending. A denylist on the Hub would be too late: by then the content has
left. Transcripts, diffs and file contents are not on it (§3).

### Snapshots

Paired desktops report **Project and Workflow snapshots with local paths
stripped** (decision 65). A `Snapshot` is one struct carrying the instance's
Projects, Workflow versions and run-shape settings together. **The Hub holds one
row per instance and each snapshot replaces it wholesale**, so there is one
current copy and nothing to merge. A snapshot is sent after `hello` and whenever
its content changes. The Hub answers it with no frame: the instance keeps no
cursor for snapshots, and one that was lost is made good by the next.

The desktop sends its first snapshot after the connection's first `CursorAck`,
which proves the Hub processed `hello`, and only while `read` is pinned and
its toggle is on (§7). A
write to a Project or a Workflow wakes a rebuild, and the rebuilt snapshot is
sent unless it equals the last one sent on that connection; a new connection
always sends its first. One that cannot be built, or whose frame is over the
4 MiB of §11 *The socket's carried defaults*, is logged and not sent, and the
session goes on. When the rebuild runs, what it sends, and what it leaves stale
are carried defaults (§11 *The desktop socket's carried defaults*). The builder is
[`domain/hub_socket/snapshot.rs`](../crates/demeteo-core/src/domain/hub_socket/snapshot.rs);
what one connection sends of it is
[`adapters/hub/socket/snapshot_send.rs`](../crates/demeteo-core/src/adapters/hub/socket/snapshot_send.rs).

**Features and harnesses ride the snapshot** ([decision 70](DECISIONS.md#1-the-locked-decisions)).
`features` names the instance's newest 100 Features that are neither archived
nor deleted, each as `{id, project_id, title, workflow_id, workflow_version}`:
the title scrubbed of secrets as a step document is and cut to 200 characters,
the version the number of the one the Feature pins. Beside each Workflow's
latest version, `workflows` then carries the versions those Features pin, at
most 20 of them, newest Feature first, so the Hub can name a run's steps from
the graph that run actually follows. `harnesses` is the sorted list of the
agent registry's kinds and nothing else about them. Both are `#[serde(default)]`,
so a Hub reads an older desktop's snapshot; an older Hub refuses a newer one,
so the Hub ships first (§11 item 8).

A snapshot carrying them was chosen over a per-Feature frame on the event
stream: the snapshot is already replaced wholesale, already gated on `read`,
and already rebuilt on a wake, where a new frame would be a second channel the
Hub merged with no cursor. Starting a run writes nothing the snapshot
decorators watch, so the session wakes the rebuild itself on the **first event
of a Feature it has not read before on that connection**. A new run's events
therefore reach the Hub up to one debounce window before its title does, and
its views show it untitled meanwhile. A Feature's title edited after its first
event is sent with the next snapshot something else wakes.
[`domain/hub_socket/snapshot.rs`](../crates/demeteo-core/src/domain/hub_socket/snapshot.rs)
holds the caps; the Hub's lookups are
[`api/run_labels.rs`](../crates/demeteo-hub/src/api/run_labels.rs).

These are what a Hub-authored `start_feature` names (§6) and what the
future runner dispatch would build a `RunSpec` from (§10). The Hub never authors a
Project or a Workflow: **rejected, because it is a second editor and a second copy
that drifts from the desktop's.** **Workflows read from repo files** were rejected
for being a new concept — a Workflow in a Project's tree would need its own
versioning story, and [decision 38](DECISIONS.md#1-the-locked-decisions) already
pins a Feature to a Workflow *version*.

The wire types cannot strip a local path themselves: a Workflow version's
`steps_json` and `definition_json`, and a Project's `remote_url`, are strings
the protocol crate carries unchanged. The desktop snapshot builder strips them
before it constructs a snapshot:

| Field | What the builder removes |
|---|---|
| Every key named `cwd`, at any depth, in both Workflow documents | the whole field — it names a directory in the instance's tree |
| `remote_url` | the whole userinfo (`https://user:token@host/…` → `https://host/…`); a Git token never leaves an instance (§3) |

`remote_url` is not read from git: it is `https://{host}/{repo_path}`, built
from the Project's first Repository and the provider that Repository names.
It is sent as nothing at all when either is missing, when `repo_path` is empty,
or when what is left after the strip still holds an `@`, whitespace or a
control character. A Workflow document that does not parse omits its whole
version.

A local path a step names anywhere but `cwd` — in its `command` or a prompt
template — is not stripped, and those fields are not safe either: §11 item 11.

### Queued requests

A request for an instance that is offline waits on the Hub. When the instance
sends `hello`, the queued requests are delivered after the `hello`'s ack,
**before** any new one, in the order they were queued. A request queued while
the instance is connected goes out on that connection.

**An expired request is never sent.** A request is deliverable strictly before
its `expires_at`, by the Hub's clock read at the moment it would be sent. At or
past it the Hub marks the request `expired` and sends no frame for it.

**Rejected: delivering it anyway and leaving the verdict to the instance
alone.** A request would then take effect past its expiry on any instance whose
clock runs behind. The instance's own check (§6.1) stands beside the Hub's and
is not replaced by it: its clock is the one it trusts, so a request the Hub sent
as live can still come back `refused: expired`, and that is recorded like any
other outcome. A request takes effect only if it is live by both clocks.

**Delivery is at-least-once.** A request stays queued until it is answered. One
that was sent and not answered before the connection dropped is sent again on
the instance's **next** connection — the same envelope, the same `request_id`,
the same signature, since it is signed once (§6.1). It is not re-sent on the
connection that already carried it. An instance can therefore be sent a request
it has already acted on; it refuses that copy as `replayed` (§6.1), and is right
to.

**The first recorded outcome is kept.** A `RequestResult` is recorded against
the request its `request_id` names, among this instance's requests only. A
second result for the same request changes nothing, so a `refused: replayed`
that follows a recorded `accepted` does not replace it. The same echo can be the
*only* result to arrive, when the first was lost outright: it is then recorded,
and what it says is that an earlier delivery's result never reached the Hub —
not that the instance declined. A result that names no request of the instance
changes nothing and does not close the socket. No frame acknowledges a result.

`expired` is the Hub's verdict that it will not send the request (again), and it
yields to the instance's: a late result for a request that was sent before it
expired replaces the mark. One that was never sent keeps it.

**There is no sweep.** A request is judged when its instance's queue is read —
on `hello`, and whenever a new request is queued for a connected instance. One
for an instance that never reconnects stays `queued` past its expiry. `queued`
means "not answered and not judged yet", never "deliverable", and a view over
the queue has to apply the expiry rule itself.

---

## 6. Requests

### 6.1 The envelope

**Every Hub → instance request is one envelope, signed by the Hub.** It travels as
JSON in the shape `SignedRequest` in `crates/demeteo-hub-protocol` parses:

<!-- EXAMPLE: new -->

```
{
  "v":           1,
  "request_id":  "Zk3m0R8pQnVx2tL5aYw9bA",   // 128 random bits, base64url
  "instance_id": "i-7f3a9c",
  "issued_at":   1791019800,                  // Unix seconds
  "expires_at":  1791020400,
  "payload": {
    "kind":      "start_feature",
    "body":      { …, "max_budget_cents": 1250, … }   // per kind, below
  },
  "signature":   "…"                          // the Hub's, base64url
}
```

**What is signed is not the JSON.** It is the pre-image `SignedRequest::signing_bytes`
builds: the label `demeteo-hub/v1/request`, then every field above except
`signature`, each length-prefixed or fixed-width and big-endian. The Hub builds
those bytes from the request it is about to queue and signs them with its
request-signing key. The instance parses the JSON, rebuilds the same bytes from
the parsed request, and verifies the signature against the key it pinned at pairing
(§4). The byte layout is specified once, in the module rustdoc of
[`canonical.rs`](../crates/demeteo-hub-protocol/src/canonical.rs) and on
`signing_bytes`, and is not repeated here.

**The signature is Ed25519 over `signing_bytes()` itself.** There is no separate
SHA-256: the message handed to the signer, and to the instance's verifier, is the
pre-image as it stands. Ed25519 hashes its own input and Transit signs what it is
handed, so a digest in between would be one more step for the two ends to disagree
on. `signature` is the 64 signature bytes as unpadded base64url. The desktop's
verifier is [`verify.rs`](../crates/demeteo-core/src/domain/hub_request/verify.rs).

**A request is signed once, when it is queued**, and never again. The envelope is
stored with its signature, and every delivery — a redelivery included (§5) — sends
that stored envelope. If the key service cannot sign, nothing is queued.
**Rejected: signing at send time**, which puts a call to the key service in the
socket's loop and gives a redelivery a second signature to get wrong.

JSON is never signed because key order, whitespace and number spelling are not
stable across two serialisers. Instead, both ends compile against that one encoder,
so there is a single definition of the bytes and no second implementation to
disagree with it. That holds only while every signed value survives a JSON round
trip bit-for-bit, so **every signed number is an integer**: the budget is
`max_budget_cents`, never dollars as a float. A parsed float need not reproduce the
bits the Hub signed, and a request whose bytes differ fails verification and is
dropped without a `result`. Do not add a float to any payload.

| Property | Rule |
|---|---|
| Signature | the Hub's request-signing key, the one pinned at pairing (§4), over `signing_bytes`. A request that does not verify is dropped without a `result` |
| `request_id` | **single-use**. The Hub mints 128 random bits per request and spells them as unpadded base64url; a redelivery carries the same id. The instance records every id it accepts in its own SQLite (an id is not a secret) for at least as long as it can still be unexpired, and refuses a repeat, including across a restart |
| `issued_at`, `expires_at` | the Hub's clock when the request was queued, and **600 seconds** after it — a carried default (§11). `expires_at` is checked against **the instance's clock**: an expired request is answered `expired` and has no effect. The Hub applies the same bound to its own clock before it sends (§5) |
| `instance_id` | the id the pairing response gave the instance (§4), spelled the same way. It must equal this instance's id, so one instance's request cannot be replayed at another |
| Order of checks | signature → scope → `instance_id` → expiry → id unseen → kind-specific checks. The signature is first because every later check is answered, and an answer to a request that does not verify would be the forger's. The id is **spent once signature, instance and expiry pass**, before the effect, and is **not refunded** if a later check refuses: a retry is a new request with a new id |

Refusal is the default. An unknown `kind`, a kind whose scope is off, or a body
that does not parse is answered `refused` and does nothing.

#### On the desktop

The rules above are decided in
[`domain/hub_request/`](../crates/demeteo-core/src/domain/hub_request/mod.rs),
synchronously and without I/O, and carried out by
[`application/hub_requests.rs`](../crates/demeteo-core/src/application/hub_requests.rs),
which the socket hands every request frame to. What a request comes to:

- **A `request_id` longer than 64 bytes** (the Hub's are 22 chars) is treated
  as no header at all, whether or not the body parses: neither answered,
  logged nor consumed. It is checked before the signature, which is what
  would vouch for it.
- **A body that does not parse** is answered from its header alone, unverified
  and unconsumed: `unknown_kind`, or `malformed_body` for a known kind. It is
  the one answer given to a body that was not verified. A body this build
  cannot read has no `signing_bytes` to verify a signature over, and left
  unanswered it would sit pending on the Hub until it expired. The cost is
  accepted: a forged body reusing a pending request's id can fix the Hub's
  outcome for that id as refused, but it runs nothing and spends no id, and
  only the Hub's own socket delivers frames to the desktop.
- **A scope is in force** only if pairing approved it and the desktop's toggle
  for it is on (§7). `read`'s toggle defaults on, and needs no request to
  refuse: off, the socket sends no events or snapshots (§7). A scope that is
  off is answered `scope_off`, after the signature verifies.
- **A request that does not verify gets no `result`.** It is logged as
  `dropped`, with the reason `signature`, and its id is not spent. An instance
  holding no pins drops every request the same way.
- **A `gate_decision` with `gates` on, and a `passkey_revocation`, is
  consumed and answered** once its WebAuthn assertion has been judged against
  the pinned set, in the order of §8 *What the desktop verifies*. A
  `gate_decision` with `gates` off is refused `scope_off` before its
  assertion is read. The passkey kinds need no scope. These two are handled
  one at a time, from loading the pinned set to the counter or set write
  their assertion commits, so each is judged against the set the one before
  left; that hold is released before a gate is decided, which can start the
  next step.
- **A `passkey_endorsement` is consumed and refused `unknown_kind`**, its
  assertion unread and nothing loaded for it, so a pinned set that fails to
  load does not defer it (§11 item 20).
- **A request the desktop cannot judge is deferred**: not consumed, not
  answered, logged as `deferred`. If the pins or the policy fail to load, a
  `start_feature`'s 24-hour committed total (§7) fails to load, the pinned
  passkeys a `gate_decision` or a `passkey_revocation` is judged against, or a
  `gate_decision`'s step, gate and run owner, fail to load, or the id cannot
  be recorded, nothing is answered, and the redelivery is judged afresh. Each
  of these is loaded before the id is recorded, so a request deferred for one
  does not spend its id. The Hub delivers it again on the next connection
  until it expires.
- **The id is recorded by one atomic insert** into `hub_consumed_requests`, so
  two deliveries cannot both read as fresh. Every id that insert checks also
  prunes the rows whose `expires_at` is more than ten minutes past. A pruned
  id cannot come back as fresh: one at or past its `expires_at` by this
  desktop's clock is refused `expired` before it is ever recorded, whatever
  the Hub's clock said when it was sent. The ten minutes cover this desktop's
  clock stepping backwards after a prune, which would make a pruned id live
  again. A prune that fails is logged and changes nothing about the request.
- **`start_feature`** is judged against the ceilings (§7), then its Project and
  Workflow are looked up (`unknown_project`, `unknown_workflow`), then the
  harness and identifier refusals `set_step_assignment` applies are applied to
  its feature-wide `agent_kind` and `model` and to every step override:
  `harness_not_registered` for a harness this desktop does not have, then
  `malformed_body` for a value that starts with `-` or holds whitespace or a
  control character. The attachments it declares are then held to the limits
  the bootstrap applies (`MAX_ATTACHMENTS_PER_FEATURE`, `MAX_ATTACHMENT_BYTES`
  and `MAX_AGENT_ATTACHMENT_BATCH_BYTES` in
  `crates/demeteo-core/src/application/attachments.rs`) before any is fetched:
  too many, one declared empty or too large, or a total too large is
  `attachment_mismatch`. The Hub deletes each attachment as it hands it over
  and the fetcher buffers up to the declared size, so these cannot wait for
  the bootstrap. Only then is every attachment fetched once (§6.3). A fetch that fails or does not match is
  `attachment_mismatch`. Each attachment is then typed from its content, since
  the reference carries no type: a PNG, JPEG, GIF, WebP, TIFF or PDF signature,
  else UTF-8 text without a NUL byte as `text/plain`. A `%PDF-` past offset 0
  makes it a PDF only when the content is not such text. Content of any other type
  is `attachment_mismatch` too, answered before anything is launched, because
  the bootstrap would refuse to commit it after an accept. The run is launched by the call the desktop's own
  launch button makes, as a local run, and the Feature is stored with
  `launched_by = 'hub'`. **`max_wall_clock_secs` is ignored**: a local run has
  no wall-clock cap to give it. The request is still accepted, and its log row
  records that the cap was ignored.
- **A launch that fails after every check passed is answered
  `malformed_body`.** `RefusalReason` has no generic failure. The id is already
  spent, so leaving it unanswered would only get a `replayed` from the
  redelivery. The error's detail, scrubbed of secrets and with the request's own
  title and description cut out, stays in the desktop's own log row. A cancel
  or an assignment that fails in its call is answered the same way, and so is
  a passkey counter or trusted-set write that fails after its assertion
  verified.
- **`cancel_feature`** answers `unknown_feature` for a Feature the desktop does
  not hold. Otherwise it is the desktop's own cancel, sent to the runner when a
  `demeteo-runner` owns the run.
- **`set_step_assignment`** resolves the step to its latest execution
  (`unknown_step`), then applies the refusals of
  [decision 54](DECISIONS.md#54--mid-run-assignment-detail) before the
  desktop's own assignment call.
- **One start is handled at a time**, so two starts cannot each be judged
  against a daily total that misses the other. The handler holds that, not the
  socket session, so a start that arrives on a socket that reconnected or
  re-paired while another was in flight waits for it too. Which of two waiting
  starts goes next is not promised. **Cancels and assignments are not held
  behind a start**: neither reads that total, and each id is spent by one
  atomic insert, so a `cancel_feature` is answered while a start is still
  fetching. **A start's fetches share one deadline**, `START_FETCH_DEADLINE`
  in `crates/demeteo-core/src/application/hub_requests.rs`, however many
  attachments it names; a start that overruns it is `attachment_mismatch`
  with nothing launched, as a fetch that failed is.
- **A result waits for a connection only as long as its socket runs.** One
  that missed the connection its request came on is sent on the socket's next
  connection, but every start of the socket — the app starting paired, or
  `hub_pair` — begins with none queued, so a restart drops it. A redelivery
  over the same device key is then refused `replayed`, and since that is the
  only result the Hub receives it is the outcome it keeps (§5 *Queued
  requests*), for a run that did start. After `hub_disconnect` the next
  pairing is a new instance (§4 *Revocation*), so the request is never
  delivered again and stays unanswered on the old one. The queue is not held
  for the process instead, because after a re-pair it would deliver one Hub's
  results to another.
- **A request already being handled outlives `hub_disconnect`.** Its task is
  not tracked by the socket, so it runs to completion — a start still
  launches its run — and its result is dropped. It is not aborted: a start
  stopped between spending its id and launching its run would leave a spent
  id with no run and no answer.

**Every request leaves one row in the activity log**, `hub_request_log`, a
redelivery included; only a frame with no readable header, or with an
over-long unverified id, leaves none. A row holds the request id, the kind (`unknown` for a body
that did not parse), the scope, the outcome (`accepted`, `refused`, `dropped` or
`deferred`), the reason, the Feature it named or started, and when it was
received. It holds no title, description, attachment content or token, and none
of it reaches the Hub, which is told a `RefusalReason` and nothing more. The
`hub_request_log` command reads it, newest first, at most 500 rows a read. A row that fails to append is
warned in the desktop's log and the answer is still sent. The row is the
desktop's record, not a condition of the answer: once an id is spent,
withholding the answer would only make the redelivery read `replayed`, and a
refusal reached before it is spent would be given again unchanged.

### 6.2 The six kinds

| Kind | Scope | Effect on the instance |
|---|---|---|
| `start_feature` | `spend` | start a Feature on a named Project and Workflow snapshot, with a description, optional attachments (§6.3) and optional per-step assignments. Refused locally if it would exceed a ceiling (§7) |
| `cancel_feature` | `spend` | cancel a running Feature. A `spend` request because it is run control, not an approval |
| `set_step_assignment` | `spend` | re-point one step's agent, model or effort mid-run. Exactly [decision 54](DECISIONS.md#54--mid-run-assignment-detail): tier 1 only, refused in `running` and `verifying`, refused for a kind that spawns no agent, refused for a harness the build has not registered |
| `gate_decision` | `gates` | decide a pending Gate. Carries a WebAuthn assertion; §8 |
| `passkey_endorsement` | none — authority is an assertion | add a passkey to the pinned set; §8. Refused `unknown_kind` by the desktop, assertion unread (§11 item 20) |
| `passkey_revocation` | none — authority is an assertion | remove a passkey from the pinned set; §8 |

`cancel_feature`'s scope was this document's assignment, not an interview answer;
the session API (§6.4) settled it as `spend` (§11 item 3). The two passkey kinds
are not scope-gated because a scope toggles *what the Hub may ask for*, and trust in a key is not something the Hub asks for: it is
granted only by a key the instance already trusts.

### 6.3 Attachments

An attachment is something the user attached to a Feature in the browser. It is the
one piece of user-authored content the Hub holds, and its lifecycle is built so the
Hub holds it as briefly and as opaquely as it can.

1. **Upload.** The browser sends the file to the Hub over TLS, as the raw body
   of `POST /v1/instances/{instance_id}/attachments` (§6.4). It needs `spend`, the
   scope of the `start_feature` that will name it. A body over the configured
   size limit is refused `413 attachment_too_large` before Transit is asked for
   anything. The answer is the attachment's id, its size, the SHA-256 of its
   content and when it expires.
2. **Encrypt.** The Hub encrypts it before it is stored, as an envelope under
   OpenBao Transit: a per-attachment data key, itself wrapped by Transit, with the
   plaintext key discarded. The content and its SHA-256 are each sealed under that
   key with AES-256-GCM, bound to the attachment's id, tenant and instance, so one
   row's ciphertext does not open as another's. Postgres holds ciphertext and a
   wrapped key, never plaintext and never a plaintext digest: a digest beside
   the ciphertext would let anyone who reads the database confirm a guessed file
   without the key. The columns are in §9.
3. **Reference.** The `start_feature` body names the attachment by id, size and the
   SHA-256 of the plaintext. No attachment bytes ride in an envelope. `size` is
   the content's length in bytes and `sha256` is the lowercase hex of its digest,
   the spelling the desktop names a stored attachment by
   ([`attachment_store.rs`](../crates/demeteo-core/src/ports/attachment_store.rs)).
   The Hub opens the sealed digest to sign it, and binds each attachment to the
   request in the transaction that queues it: either the request is queued and
   every attachment it names is bound to it, or nothing is. A dispatch may name
   only the tenant's own attachments of that instance that are neither bound nor
   expired, at most `MAX_ATTACHMENTS_PER_DISPATCH` (10) of them, each once.
4. **Fetch once.** The instance fetches it **before it answers the request**, with
   its access token (a DPoP-bound request to `GET /attachments/{attachment_id}`,
   admitted as the socket is, §5, with a proof for that one URL). The Hub
   unwraps the key through Transit, opens the content, deletes the row, and
   answers the plaintext. A fetch is served only while the request it is bound to
   is unanswered and before its `expires_at` by the Hub's clock, so an instance
   that answers first has nothing left to fetch. The instance checks the SHA-256
   and refuses the request with `attachment_mismatch` on a mismatch. It then
   types the attachment from its content, as the Hub serves every one as
   `application/octet-stream`, and content of a type it does not accept as an
   attachment is `attachment_mismatch` as well.
5. **Delete.** A completed fetch consumes the attachment: the row is deleted
   before the response is sent, and a second fetch answers `404` as if it never
   existed. The delete re-checks that the request is still queued and
   unexpired, so a request answered or expired while the fetch was opening the
   row gets `404` and no content. **There is no retry after a completed response.** A transfer
   interrupted after the delete loses the attachment, and the instance refuses
   the request; a retry is a new upload and a new dispatch. A fetch that fails
   *before* the delete — the key service sealed or unreachable, a row that does
   not authenticate — leaves the row, to be fetched again while the request is
   live. A request whose outcome is recorded — refused, cancelled or done —
   deletes its unfetched attachments in the transaction that records it. One
   that passes its expiry unanswered makes them unfetchable at once, and the
   sweep deletes them at the request's expiry.

**Every attachment carries an expiry, and a sweep deletes it.** A staged
attachment expires its TTL after the upload (`DEMETEO_HUB_ATTACHMENT_TTL_SECS`,
default 3600 seconds); a dispatch that names one past it is refused
`409 attachment_unavailable`, or `404 not_found` once the sweep has deleted it. Binding it moves its expiry to its request's, 600
seconds after queueing (§6.1), whatever the TTL. Every
`DEMETEO_HUB_ATTACHMENT_SWEEP_SECS` (default 60) the Hub deletes every row at or
past its expiry, staged or bound, tenant by tenant. That is unlike the request
queue, which has no sweep (§5): a request row is kept as the record of what was
asked, and an attachment is content, held for no longer than its request can be
delivered. The size limit is `DEMETEO_HUB_ATTACHMENT_MAX_BYTES` (default 25 MiB,
26,214,400 bytes); the Hub buffers a whole upload and a whole fetch in memory.
Each must be a positive integer, and the Hub refuses to start on any other value.
The lifecycle's rules as the code keeps them are in the module rustdoc of
[`attachments/mod.rs`](../crates/demeteo-hub/src/attachments/mod.rs).

The Hub is not trusted with the attachment's integrity, which is why step 4 checks
a hash the envelope carries and the envelope is signed.

### 6.4 The session API

**What a signed-in browser reads of its fleet, and the only way a request gets
queued.** Every route is under `/v1`, needs a session, and is not on the
pre-session list, so each `POST` is behind the same `Origin` and CSRF guard as
the rest of `/v1`. The tenant, read-scope and no-path rules are stated once, in
the module rustdoc of [`api/mod.rs`](../crates/demeteo-hub/src/api/mod.rs).

| Method and path | Scope | Answer |
|---|---|---|
| `GET /v1/fleet` | `read`, per instance — withheld fields, not an error | `200`, every instance with presence, labels, scopes, `live_runs`, `needs_you`, `harnesses` and `project_count` |
| `GET /v1/instances/{instance_id}` | `read`, per field | `200`, the instance, its projects as `{id, name, remote_url}`, its `workflows` (graph-only, below), its `run_shape`, its `harnesses` and its live runs |
| `GET /v1/runs?instance=&status=` | `read` | `200 {runs}`; `status` is `running`, `completed`, `failed` or `cancelled` |
| `GET /v1/instances/{instance_id}/runs/{feature_id}` | `read` | `200`, status, title, project, the pinned `workflow` graph, steps, ticket progress, pending gates and events in offset order |
| `POST /v1/instances/{instance_id}/attachments` | `spend` | `201 {attachment_id, size, sha256, expires_at}`, an attachment staged (§6.3); the body is the file's raw bytes |
| `POST /v1/instances/{instance_id}/runs` | `spend` | `202 {request_id, expires_at}`, a `start_feature` queued, binding the attachments its `attachment_ids` name |
| `POST /v1/instances/{instance_id}/runs/{feature_id}/cancel` | `spend` | `202`, a `cancel_feature` queued |
| `POST /v1/instances/{instance_id}/runs/{feature_id}/steps/{step_id}/assignment` | `spend` | `202`, a `set_step_assignment` queued |
| `POST /v1/instances/{instance_id}/gates/challenge` | `gates` | `200`, a held challenge and the `navigator.credentials.get` options over it |
| `POST /v1/instances/{instance_id}/gates/decision` | `gates` | `202 {request_id, expires_at}`, a `gate_decision` queued |
| `GET /v1/instances/{instance_id}/requests/{request_id}` | none beyond owning the instance | `200`, the request's kind, derived status, `deliveries`, `expires_at` and `answered_at` |

**An instance's workflows are graph-only.** `workflows` holds one entry per
`workflow_id`, at its highest `version`, as `{workflow_id, version, name,
graph}`, ordered by `name`. `name` is the definition's own, or the
`workflow_id` when it has none. `graph` is `{nodes: [{id, type, title,
position}], edges: [{from, to}]}`, read from `definition_json`, and `null` when
the version has no such document or it does not parse; the Hub does not derive
one from `steps_json`. No node `config`, `retry` or `join`, no edge `when`, and
neither `steps_json` nor `note` reaches the browser: the browser needs only the
shape to choose a workflow and name its steps, and §11 item 11 leaves open
whether that text may reach the Hub at all. `run_shape` is each project's
dispatch defaults — agent kind, model, effort, workflow id and maximum budget in
cents; `default_loop_iterations` is left out, since a dispatch has no field for
it. Both follow the read-scope rule of `projects`: `null` without `read`, `[]`
until a readable snapshot arrives. The projection is
[`api/snapshot_view.rs`](../crates/demeteo-hub/src/api/snapshot_view.rs).

**The attachment fetch is not a session route.** `GET /attachments/{attachment_id}`
is outside `/v1`, beside `/socket` and `/token`, because its caller is an
instance, which sends no `Origin` and holds no session. It takes the instance's
access token and one DPoP proof whose `htu` is that attachment's URL (§6.3 step
4), and answers `200 application/octet-stream`, once. A credential fault is
`401 invalid_token` with `WWW-Authenticate: DPoP`, as on the socket. Every other
miss — an id of another instance or tenant, one not bound to a request, one whose
request is answered or expired, one already fetched or never minted, one that
is not a UUID — is the same `404 not_found`. A key service that cannot unwrap is
`503 key_service_unavailable`, and the row stays.

**Which data needs `read`.** What the Hub holds of its own about an instance —
presence, the labels its `hello` gave, the scopes it last reported — and the
status of a request queued for it are shown without `read`. What the instance
reported, its run mirror and its snapshot, is shown only while it reports
`read` on. The fleet and the instance detail answer such an instance with
`read_granted: false` and its mirror- and snapshot-derived fields `null`; the
unfiltered runs list passes over its Features; a run detail, or a runs list
filtered to it, is refused. A route that queues needs its kind's scope alone,
not `read` beside it: the set is flat (§7). None of this is enforcement (§7).

**Order of checks.** Once a body parses as JSON of its shape: ownership, then
scope, then what the body says, then the work. An upload's size is what its body
says, judged before any call to the key service; a dispatch's attachments are
resolved after the rest of its body and before it is signed. An instance, run or request that
is another tenant's answers `404 not_found` exactly as one that does not exist,
and so does an id that does not parse. A missing scope is refused before the
body is judged, so it queues nothing and holds no challenge.

**Errors** are `{"error": code}` and nothing else, with one exception:
`scope_not_granted` also names the scope, as
`{"error": "scope_not_granted", "scope": "spend"}`.

| Status | `error` | When |
|---|---|---|
| 404 | `not_found` | the instance, run, request or attachment is not the tenant's, or its id does not parse; an attachment of another of the tenant's instances is not found too |
| 409 | `scope_not_granted` | the instance last reported the route's scope as off; `scope` names it |
| 400 | `invalid_request` | a body that does not parse or has an unknown field, a U+0000 in any string, an id over 256 bytes (`MAX_FEATURE_ID_BYTES`), an unknown `status` filter, an `attachment_ids` that repeats an id or names more than `MAX_ATTACHMENTS_PER_DISPATCH` |
| 413 | `attachment_too_large` | an upload over `DEMETEO_HUB_ATTACHMENT_MAX_BYTES` (§6.3); nothing is stored |
| 409 | `attachment_unavailable` | a dispatch naming an attachment of the tenant's instance that is already bound to a request or has expired; nothing is queued and no attachment is bound |
| 409 | `challenge_mismatch` | a decision whose `request_id` is not the one its challenge minted |
| 400 | `assertion_malformed` | an assertion that fails the structural check below |
| 400 | `ceremony_expired` | a decision naming a challenge that is not held — never issued, already spent, evicted, held for another session or user, or whose request has expired |
| 503 | `key_service_unavailable`, `too_many_ceremonies` | the envelope could not be signed, or an attachment's data key could not be minted or unwrapped; the Hub holds as many challenges as it will |
| 500 | `internal` | a store failure |

**A request's status is derived, never stored.** The store holds `queued`,
`expired` and `answered` (§9); the route answers one of five, read off the row,
its outcome and the Hub's clock:

| Status | When |
|---|---|
| `queued` | not answered, never sent, and still deliverable |
| `delivered` | not answered and **sent at least once**; also a recorded `refused: replayed`, which says an earlier delivery's result was lost, not that the instance declined (§5) |
| `accepted` | the instance's result was `accepted` |
| `refused` | the instance's result was `refused`, with `reason` beside it — `expired` among them when the instance's own clock judged it |
| `expired` | the Hub marked it so, or it is still `queued` at or past `expires_at`: there is no sweep (§5), so the route applies the expiry rule itself |

`answered_at` is `null` unless the status is `accepted` or `refused`.

**A gate is decided in two steps**, because the instance recomputes the
challenge from the envelope it receives (§8), so the envelope's `request_id`
and `expires_at` have to exist before the human signs.

1. **Challenge.** The body names `feature_id`, `step_execution_id` and
   `decision`. The Hub mints the request id and `expires_at` now — 600 seconds
   on, the request lifetime of §6.1 — builds the pre-image with the protocol
   crate's `gate_challenge`, and holds all six fields as a ceremony under the
   passkey that signed the session in. The answer carries the ceremony id, the
   request id, `expires_at`, the pre-image as `gate_challenge`, and WebAuthn
   options whose challenge is its SHA-256, allowing the tenant's trusted
   passkeys.
2. **Decision.** The body carries the ceremony id, the request id and the
   assertion only. A body that names `feedback` is refused `invalid_request`
   before the held challenge is taken, so that challenge is not spent. The Hub
   takes the held challenge, refuses a different request id, checks the
   assertion's shape, refuses a request already past `expires_at`, and queues
   a `gate_decision` whose `instance_id`, `request_id`, `expires_at`,
   `feature_id`, `step_execution_id` and `decision` are **the held
   challenge's, never the body's**, and which carries no `feedback`. The
   protocol keeps that field, but the challenge does not bind it (§11 item 1)
   and the desktop discards it (§8), so the Hub collects none rather than let
   its sender believe it arrived. A challenge taken and refused is spent, so a
   retry starts from step 1.

**The Hub's assertion check is structural only.** Every field is unpadded
base64url, the authenticator data is long enough to hold its counter, and the
client data is a `webauthn.get` over the held digest. It checks no signature,
origin, RP ID or flags: the passkeys the Hub holds are the ones it signs
tenants in with, not an instance's pinned set, so a signature it accepted would
prove nothing. It is an early filter for an assertion no instance could
accept. **The instance is the authority** (§8, *What the desktop verifies*),
and an assertion the Hub passed may still come back `refused`.

A held challenge lives in process memory, like a passkey ceremony: for 300
seconds at most, lost on a restart, and seen by one replica only (§11 item 14).
Its request's `expires_at` is fixed at step 1, so a decision submitted late has
less than the full 600 seconds left to be delivered.

**What a run is called.** A run summary, a `needs_you` entry and a run detail
carry `title`, `project_name` (and, on a summary and a detail, `project_id`),
read from the instance's last snapshot (§5, *Features and harnesses*). A
summary adds `step_id`, the Step of the latest `step_started`, and its
`tickets`; a summary's `step_id`, a pending gate and a detail's steps carry a
`step_title`. A Step is titled only from the graph of the Workflow version the
Feature pins, and a detail's `workflow` is that graph as a schema-v2 document
cut to `{schema_version, id, name, nodes: [{id, type, title, position}],
edges: [{from, to}]}` by the graph-only rule above. Each is `null` when the
snapshot gives none — a Feature older than the newest 100, a Project it does
not name, a pinned version it did not send — and nothing is read from another
version or inferred from ids.

**What the views cannot show yet.** `cost_cents` on a run is always `null`:
no protocol message reports a cost (§5). It fills in when one does; until then
nothing is inferred.

---

## 7. Scopes

| Scope | Allows | Requests |
|---|---|---|
| `read` | the instance sends events and snapshots; its toggle defaults on | — |
| `spend` | the Hub may start, cancel and re-point runs that spend money | `start_feature`, `cancel_feature`, `set_step_assignment` |
| `gates` | the Hub may relay a gate decision | `gate_decision` |

The set is **flat**, like [MCP's](MCP_INTEGRATION.md#6-scopes): `spend` does not
imply `read`. The scopes are split by consequence, as MCP's are, so a consent
screen says what an action *costs*.

**The instance toggles and enforces every scope.** Pairing records what the user
approved (§4); the toggle on the desktop is the later word. A request for a scope
that is off is refused locally. **No Hub action can turn a scope on** — not a
request, not a re-pairing prompt the Hub raises, not a setting the Hub reports.
`hello` states the scopes that are on now, so the Hub can tell a refusal is
coming; it does not ask permission. **Turning `read` off** stops the socket
relaying events and snapshots: a running socket restarts at once and sends
only its `hello`. It is the one toggle that acts without a request to refuse.

**`granted_scopes` on the Hub is what `hello` last said.** Pairing writes what
the user approved, and the first `hello` replaces it with what the desktop says
is on now; every later `hello` does the same. The Hub's copy is a report, one
`hello` old, and the Hub enforces nothing with it. It stores events and
snapshots from an instance that did not report `read`: whether to send them is
the instance's decision.

**The Hub's uses of that copy are informational.** Queueing a request whose
scope the instance last reported as off is declined — `409 scope_not_granted`
naming the scope, and nothing is queued (§6.4) — because the request would only
come back `refused`. That spares a round trip and is not the enforcement: the
instance checks its own toggles against every request it receives (§6.1), and
nothing may come to depend on the Hub's gate holding. `passkey_endorsement` and
`passkey_revocation` need no scope (§6.2) and are queued for an instance with
every scope off. The session API also withholds what the instance reported
while `read` reads as off (§6.4); what it stored is unchanged.

> **Do not let the Hub widen a scope.** Any message that would "enable" `spend` or
> `gates` from the Hub side is the Hub granting itself authority, which is the one
> thing the toggle exists to prevent. A scope turns on in one place: the
> instance's own Settings.

**Not offered:** a `logs` scope (§3), and MCP's `configure`. `configure` changes how
runs are shaped ([decision 48](DECISIONS.md#1-the-locked-decisions)); the Hub starts
and steers runs but does not edit a Project's settings.

### Local ceilings

A Hub-started run is bounded by two numbers **set on the desktop**: a **per-run
maximum cost** and a **rolling 24-hour total**. A `start_feature` over either is
refused locally, with a `result` that names which. The Hub cannot read them as
permission and cannot change them.

**Rejected: toggle-only scopes.** With `spend` on and no ceiling the cap would be
whatever the request carried, and a cap the Hub supplies is a cap the Hub can
raise. The ceiling is the part of this design that makes `spend` a bounded risk
instead of a delegated one.

**What they count.** Both are integer cents, like the `max_budget_cents` they
are compared with (§6.1). The rolling total is the sum of the **committed caps**:
the `max_budget_usd`, in cents, of every Feature with `launched_by = 'hub'`
created in the 24 hours before the desktop's clock. It is not the cost incurred.
A `start_feature` is accepted only if its cap is at most the per-run ceiling,
refused `over_run_ceiling` otherwise, and if the total plus the cap is at most
the rolling ceiling, refused `over_rolling_ceiling` otherwise. Both bounds are
inclusive. Neither `cancel_feature` nor `set_step_assignment` counts against
either. The arithmetic is
[`ceiling.rs`](../crates/demeteo-core/src/domain/hub_request/ceiling.rs).

**Every gap refuses.** A start with no cap, or one arriving while the per-run
ceiling is unset, is refused `over_run_ceiling`. An unset rolling ceiling and
a sum that overflows each refuse `over_rolling_ceiling`. A total that did not
load is not a refusal: the start is deferred, unanswered and unspent (§6.1). No
unset value reads as "unlimited", so both ceilings default to refusing every
start, and `spend` and `gates` default to off.

**The per-run ceiling bounds the declared cap, not what the run spends.** The cap
becomes the Feature's `max_budget_usd`, which is a per-turn budget
([`budget.rs`](../crates/demeteo-core/src/domain/agent_session/budget.rs)), so a
run of many turns can spend more than its cap. Only the claude-code adapter
passes that budget on, as `--max-budget-usd`; the other adapters pass none, so
a start naming another harness has no budget enforced even per turn. Bounding a local run's actual spend is §11 item 4's remainder.

**The toggles and ceilings are the desktop's own.** They are one row of
`hub_local_policy`. The request handler is given a reader of it and no writer.
The only writer is the `hub_policy_set` command, which Settings › Hub calls
(§2); it saves through `application::hub_settings::save_hub_policy`, which
restarts a running socket when `read` flips. The policy, the activity log and
the consumed ids are not pairing state: `hub_disconnect` leaves them, and pairing with
another Hub keeps the toggles and ceilings as they were.

The mockup shows no place to set a ceiling, and none for passkey management (§8).
Settings › Hub has both: the ceilings beside the toggles, and the trusted
passkeys as a read-only list.

---

## 8. Gate decisions

A Gate may be decided from the Hub. [Decision 50](DECISIONS.md#1-the-locked-decisions)
says the opposite for MCP, and both stand. The difference is not tone, it is
mechanism:

| | MCP | The Hub |
|---|---|---|
| What the caller holds | a bearer token | a request the Hub relays |
| Who holds it | an agent-reachable client | the Hub, a service |
| What authorises a decision | the token | **a WebAuthn assertion made by a human on an authenticator** |
| Who verifies | the desktop, against a grant | **the desktop, against passkey public keys it pinned** |
| Could the caller make a decision on its own | yes — a token can be used by whoever holds it | **no** |
| Decision 50 | excluded, permanently | not reopened — a separate surface |

An approval the gated party can grant is not one (decision 50's reasoning). An agent
can use a token it can reach. It cannot produce an assertion, and neither can the
Hub. **No token can approve.** A session cookie, an access token and a `gates`
scope are all necessary and none is sufficient. What makes a decision is the
assertion.

### The challenge

The WebAuthn challenge commits to six fields, in this order:

| # | Field | Source |
|---|---|---|
| 1 | `instance_id` | the envelope |
| 2 | `feature_id` | the Gate's Feature |
| 3 | `step_execution_id` | the Gate's step execution |
| 4 | `decision` | `approve` or `cancel`; see below |
| 5 | `request_id` | the envelope — so an assertion cannot outlive its request |
| 6 | `expires_at` | the envelope |

**The pre-image** is what `gate_challenge` in
[`canonical.rs`](../crates/demeteo-hub-protocol/src/canonical.rs) returns — the
same encoder as the envelope's, so the Hub that asks the browser for an assertion
and the instance that checks it build one byte string from one definition. Its
rustdoc holds the byte layout; this section does not restate it.

- It opens with the label `demeteo-hub/v1/gate` and the protocol version. The label
  names the purpose, so an assertion made for one purpose can never be accepted for
  another: the two passkey challenges open with `demeteo-hub/v1/endorse` and
  `demeteo-hub/v1/revoke` (below), and the envelope's pre-image with
  `demeteo-hub/v1/request`.
- The six fields follow in the table's order. Every string is length-prefixed, so
  no value can shift bytes into its neighbour and an id needs no character
  restrictions to be safe. `decision` is a tag byte and `expires_at` a fixed-width
  integer of Unix seconds, so neither has a spelling to disagree about.
- The challenge is the **SHA-256 of the pre-image**, passed to WebAuthn as 32 bytes
  and carried as base64url without padding. The crate returns the pre-image only;
  hashing it is the caller's job.

<!-- EXAMPLE: new -->

For `instance_id` `i-7f3a9c`, `feature_id` `f-1791004167577`, `step_execution_id`
`se-0042`, `approve`, `request_id` `Zk3m0R8pQnVx2tL5aYw9bA` and `expires_at`
`1791020400`, the pre-image is these 104 bytes, in hex:

```
0000001364656d6574656f2d6875622f76312f67617465        label
00000001                                              version 1
00000008692d376633613963                              instance_id
0000000f662d31373931303034313637353737                feature_id
0000000773652d30303432                                step_execution_id
01                                                    decision: approve
000000165a6b336d30523870516e567832744c35615977396241  request_id
000000006ac0cd70                                      expires_at
```

and the challenge is `ss9J2G8tlsYSapyixbEJ-ruL5ykKX4OTGBZeezoMZZY`. The values are
made up; the digest is real and is the test vector for the rule above.
`gate_challenge_matches_the_hub_md_vector` in
[`tests/signing.rs`](../crates/demeteo-hub-protocol/tests/signing.rs) pins the same
pre-image byte for byte, so an encoder change that would invalidate this vector
fails that test. The digest itself is computed outside the crate, which has no
hashing dependency: change the bytes and recompute it here.

### What the desktop verifies

Before a `gate_decision` has any effect, in this order, after the envelope checks
of §6.1:

1. the step execution exists and belongs to the request's `feature_id`, else
   `unknown_step`;
2. its run is not a run a `demeteo-runner` owns, else `runner_owned`. The
   desktop only mirrors such a run and holds no gate row for it, so this comes
   before the pending check, which would answer `gate_not_pending` for a gate
   that is pending on the runner, and before the assertion, so no counter is
   spent (§10);
3. its Gate is **pending**: a gate row exists for the step and the step has not
   completed, failed or been cancelled — the rule the desktop's own
   `gate_decide` applies — else `gate_not_pending`;
4. the assertion verifies, by the checks below, over the challenge the instance
   **recomputes from the envelope** and its own pinned `instance_id`. A
   credential outside the pinned set is `credential_not_pinned`, and every
   other failure `assertion_invalid`;
5. the passkey's stored counter moves to the one the assertion reported, by a
   compare-and-set. A key revoked or used meanwhile is `credential_not_pinned`;
6. the desktop's own `gate_decide` is called with `approve` or `cancel` and no
   feedback, because the assertion does not bind the request's `feedback` and
   the gate keeps feedback as the human's own words, for project memory and the
   next agent step's prompt. One it refuses is `gate_not_pending`.

An assertion verifies only if each of these holds, checked in this order:

1. each field is strict unpadded base64url within a bound checked before it is
   decoded: a credential id of at most 1023 bytes, `authenticatorData` of 37 to
   1024, `clientDataJSON` of at most 4096 and a signature of at most 128;
2. the credential id is in the **pinned set**;
3. its pinned public key is a canonical ES256 or Ed25519 `COSE_Key`;
4. the signature verifies under that key over
   `authenticatorData ‖ SHA-256(clientDataJSON)`. It is checked before
   `clientDataJSON` is parsed, so nothing the key did not sign reaches the
   parser;
5. `clientDataJSON.type` is `webauthn.get`;
6. its `challenge` decodes to the 32 bytes of the recomputed challenge;
7. its `origin` equals the **pinned `hub_origin`**, byte for byte. It is not
   `https://` followed by the RP ID: the two differ when the RP ID is a parent
   domain of the Hub's host, and for a development origin such as
   `http://localhost:PORT`. The Hub compares against its configured
   `public_origin`, and pairing pins that origin: every `htu` the Hub accepted
   was derived from it (§4);
8. `authenticatorData` carries the SHA-256 of the pinned RP ID;
9. the user-present flag is set;
10. the user-verified flag is set;
11. **the counter advances**, the rule of WebAuthn Level 2 §7.2 step 21: a
    stored and a reported count of 0 is accepted, because synced passkeys
    commonly report zero; otherwise the reported count must be strictly greater
    than the stored one. An equal non-zero count is refused as a possible clone.
    The reported count is stored once the assertion is accepted, before a gate is
    decided, and in the same transaction as an endorsement's or a revocation's
    write.

The instance recomputes the challenge instead of reading one from the Hub, so the
Hub cannot present an assertion for one decision as an assertion for another. The
Hub refuses nothing over a counter: its sign-in logs one that did not advance as a
warning and lets the sign-in through
([`auth/handlers/sign_in.rs`](../crates/demeteo-hub/src/auth/handlers/sign_in.rs)),
and its own chain of endorsements and revocations compares none. The desktop,
which is the authority for a gate, keeps one per pinned key and refuses by the
rule above.

Signatures are verified with `aws-lc-rs`, already a dependency of
`demeteo-core` and the library the Hub's verifier uses, so both ends accept the
same signatures. `ring` was proposed and is not a direct dependency: adding it is an
[AGENTS.md §6](../AGENTS.md) Gate item, and it was asked and not answered.

`decision` is `approve` or `cancel`. `redirect` carries free-text feedback that none
of the six fields binds, so a relaying Hub could rewrite what the human wrote; until
that is settled (§11) an instance refuses a `gate_decision` that is not `approve` or
`cancel`. It also never maps an unrecognised string to a verdict, because
`classify` in `domain/gate/decision.rs` cancels the run on one.

### The pinned set

The set **starts from the passkeys returned at pairing** (§4). It changes only by
two request kinds, and neither is authorised by the Hub. The desktop applies only
the second today:

- **`passkey_endorsement`** would add one. The desktop consumes and refuses every
  one as `unknown_kind` without reading its assertion (§11 item 20). The rule it
  is designed to, that of
  [decision 66](DECISIONS.md#66--hub-gate-decisions-detail), is this: it carries
  the new credential's id and public key and an assertion, **by an
  already-trusted passkey, over the new credential's id and public key**
  (challenge prefix `demeteo-hub/v1/endorse`). The new key is
  trusted when the instance verifies that assertion, not before. The new key
  must be a canonical `COSE_Key`, its id must not be pinned already — an
  endorsement never replaces a key — and the signer must be **a different key**
  from the one endorsed, so a key cannot vouch for itself. The new key is stored
  exactly as the request carried it, because the challenge commits to that
  string. `endorsement_verdict` and the store's `endorse` implement this, and no
  request reaches them.
- **`passkey_revocation`** removes one. It carries an assertion **by another trusted
  passkey** over the credential to remove (prefix `demeteo-hub/v1/revoke`). A key
  cannot revoke itself, and **the last trusted key cannot be revoked**, so no
  request can leave the instance with nothing it trusts. The target is checked
  to be pinned and not the last before the signer is looked up. What keeps two
  revocations racing each other from emptying the set is the signer: the store
  deletes the target in the transaction that advances the signer's counter by
  compare-and-set, and only while the signer's own row still exists
  (`EXISTS(signer)`), so every deletion leaves its signer behind however they
  interleave. The store's refusal to delete the last row (`COUNT(*) > 1`) is
  defence in depth against a self-revocation reaching the store, the only write
  it can catch, which `revocation_verdict` refuses first as `self_revocation`.

Both assertions bind `instance_id`, `request_id` and `expires_at` the way a gate's
does; the exact field lists are those of `endorsement_challenge` and
`revocation_challenge` in `canonical.rs`, and remain defaults (§11). The
instance verifies a revocation by the checks of *What the desktop verifies* over a
challenge it recomputes with **its own pinned `instance_id`**, so it accepts an
instance-bound assertion only, never a `"*"`-bound one. A failed check is
answered as a gate's is; a target that is not pinned is `credential_not_pinned`,
and the two revocation rules `self_revocation` and `last_credential`.
`endorsement_verdict` judges an endorsement the same way, with a signer equal to
the key endorsed `credential_not_pinned` and an id already pinned or a key that
is not canonical `malformed_body`, once a request reaches it.

**An endorsement binds what the ceremony signed, not what the user meant to
sign**, which is why the desktop refuses every one (§11 item 20). The Hub serves
the hub-web bundle that runs every passkey ceremony (§2), so a compromised Hub
can feed an `endorsement_challenge` for a key it holds to a ceremony the user
takes for a sign-in, and the authenticator shows the user only the RP. The same
substitution can still revoke one of the user's other keys. An applied
revocation's activity-log row, whose reason names the revoked id and its signer,
is the desktop's only record of it, and no view shows that log or the trusted
set yet (§2).

**The Hub keeps the same chain for itself.** After a tenant's first passkey, which
passkeys may sign in to the Hub changes only by these two assertions, and the Hub
stores each one as a record; the module rustdoc of
[`auth/mod.rs`](../crates/demeteo-hub/src/auth/mod.rs) says what that proves and
why a session alone moves nothing. Such an assertion is made at the Hub and
addressed to no instance, so its challenge binds `instance_id = "*"`, a fresh
`request_id` and an `expires_at` 300 seconds after it was minted, and all three
are stored with the record. That keeps a record re-verifiable as a signature. It
is a **carried default, not a decision**: an instance recomputes a challenge from
the envelope it received, with its own id, so nothing above has it accept a
`"*"`-bound assertion, and how it comes to trust what such a record says is open
(§11 item 9).

**Rejected: accepting each new passkey manually on every desktop.** It would hold
the property and make adding a phone a trip to every machine, which in practice is
a reason never to add one. The chain extends trust from a key the desktop already
trusts, with no visit.

**Rejected: a Hub session plus a passkey step-up only for merge-to-default and
over-budget gates.** That was the design's own shape: ordinary gates would be
approved on the session alone. The desktop would then be trusting the Hub's word
that a session is a person, and a compromised Hub or a hijacked session could
approve every gate the step-up did not cover. Every gate decision carries an
assertion, or none can be trusted.

**Rejected: read-only gates on the Hub.** Safe, and not the point of the surface.

> **A first pair is the weak moment.** The pinned set is trust-on-first-pair: an
> attacker who controls the Hub *at pairing* can hand over their own passkey keys.
> The consent screen showing the device-key fingerprint on both ends is what the
> user has to catch it with. After pairing the set changes only by a signed chain.
> **Pairing again starts the set over**: it is replaced by the passkeys the new
> pairing returns, so keys endorsed since, and every stored counter, are reset.
> That is trust-on-first-pair applied again, not a loss to repair by merging the
> two sets.

---

## 9. Storage, custody and layout

### Storage

**Postgres, plus OpenBao with its Transit engine for envelope encryption,** shipped
as Docker Compose with four services:

| Service | Role |
|---|---|
| `hub` | the axum server |
| `postgres` | tenants, passkey public keys, instance records, device-key public halves, event store, snapshots, request queue, access- and refresh-token hashes, and staged attachments as ciphertext and a wrapped key |
| `openbao` | custody; the Transit engine encrypts attachments and holds the Hub's secrets |
| `caddy` | TLS termination in front of the `hub` |

**What the socket added to Postgres** is no table: the event store, the
snapshots and the request queue date from the first migration.
[`0006_instance_socket.sql`](../crates/demeteo-hub/migrations/0006_instance_socket.sql)
adds columns to two of the tables that were there:

| Column | What it means |
|---|---|
| `instances.kind` | `desktop` or `runner`: the grant that paired the instance last (§4). Rows from before `0009_instance_kind.sql` are `desktop`, the only kind that could pair then |
| `instances.install_id` | the install id the instance's last `Hello` gave. A label the peer chose: it never picks the row a write lands on (§5), and what a changed one means is open (§11 item 17) |
| `instances.connection_id` | the one connection the instance is present through, null when it is absent. **Presence is this column being non-null**, and no column says so a second time. A connection that ends clears it only while it still holds its own id, so a displaced session ending late does not mark its successor absent |
| `instances.connected_at` | when that connection sent its `Hello`; cleared with it |
| `instances.event_cursor` | the cursor of §5, the highest event offset stored for the instance. It is written in the transaction that stores the event and is what a `CursorAck` carries; `0` until an event is stored |
| `instance_requests.deliveries` | how many times the request was sent; `0` is never sent |
| `instance_requests.last_sent_at` | when it was last sent |
| `instance_requests.outcome` | `accepted` or `refused`, from the `RequestResult` recorded for it (§5); null until one is |
| `instance_requests.refusal_reason` | the reason's wire name — present exactly when the outcome is `refused` |
| `instance_requests.answered_at` | when that outcome was recorded |

It also closes `instance_requests.status` to `queued`, `expired` and `answered`,
with `answered` holding exactly when an outcome does, so the status and the
outcome are written in one statement or the write is refused.

**What attachments added** is the lifecycle of §6.3 on the `attachments` table
the first migration made.
[`0008_attachment_lifecycle.sql`](../crates/demeteo-hub/migrations/0008_attachment_lifecycle.sql)
changes it so:

| Column | What it means |
|---|---|
| `attachments.request_id` | now **nullable**: null while the attachment is staged, set once, at dispatch, to the request that names it. The foreign key to `instance_requests` goes unchecked while it is null |
| `attachments.expires_at` | when the sweep deletes the row: the upload plus the attachment TTL while staged, the request's own `expires_at` once bound (§6.3) |
| `attachments.sealed_digest` | the SHA-256 of the content, sealed under the row's own data key. Dispatch opens it to sign the digest into the request |
| `attachments.sha256` | unchanged: the SHA-256 of the **ciphertext**, never of the content |
| `attachments.size_bytes` | unchanged: the ciphertext's length, now held non-negative. The content's length is it less the nonce and tag |

A row holds the ciphertext, the sealed digest and the wrapped data key, and
nothing from which the content or its digest could be read without Transit
(§6.3 step 2 says why the digest is sealed too). The statements are in
[`store/attachments.rs`](../crates/demeteo-hub/src/store/attachments.rs).

> **Accepted residual: the signed envelope stores the plaintext digest.** A
> `start_feature` names each attachment by the SHA-256 of its content (§6.3
> step 3), and the envelope is stored with its signature (§6.1) in
> `instance_requests.envelope`, whose rows are never deleted (§5). Anyone who
> reads the database can therefore confirm a guess of a file a tenant attached,
> after the attachment itself is gone — the confirmation the `attachments` row
> refuses. It is accepted, not solved: the instance has to check a digest the
> Hub signed, and redacting it from the stored envelope once answered would
> leave a stored row whose signature no longer verifies. A human may decide
> otherwise (§11 item 19).

**Hub server secrets live in OpenBao or are encrypted under Transit — never
plaintext in Postgres.** That includes the request-signing key (§4) and the key
material for attachments (§6.3).

**The request-signing key is a Transit `ed25519` key, and its private half never
enters the Hub.** OpenBao generates it and keeps it. The Hub's configuration holds
only the key's Transit *name* (`DEMETEO_HUB_SIGNING_KEY`), and the `KeyService`
port is the Hub's whole reach to it: `verifying_key` reads the 32-byte public
half that a pairing response hands a desktop to pin (§4), and `sign` has Transit
make a signature and return it. No table holds the key, and no process memory
holds more than its public half.

The public half is read on the first pairing that needs it, never at startup —
`/healthz` and sign-in keep working beside a sealed OpenBao — and only a
successful read is kept, for the life of the process. `sign` is reached from
one place, `enqueued_as` in
[`socket/requests.rs`](../crates/demeteo-hub/src/socket/requests.rs), which signs
an envelope as it queues it (§6.1); the session API's queueing routes (§6.4)
are its callers. A test shows the advertised public key is the one signatures
are made under. Creating the key and the policy paths it
needs are an operator's steps, in
[`deploy/hub/README.md`](../deploy/hub/README.md).

**Rejected: generating the key in the Hub and storing it wrapped in Postgres.**
It needs a table that belongs to no tenant, which the tenancy guards refuse, and
it puts a long-lived key that every instance pins into the memory of a hosted
process.

**OpenBao is MPL-2.0, the Linux Foundation fork of Vault.**

**Rejected: HashiCorp Vault.** It is under the BSL and is not open source.
**Rejected: Cosmian/Eviden KMS.** Its licensing is source-available.

> **Custody does not widen.** What OpenBao holds is the Hub's own secrets and the
> wrapped keys of attachments. It does not hold, and no later feature adds to it,
> provider API keys, harness sign-ins, local paths, transcripts or diffs: those
> never reach the Hub (§3), so there is nothing to put in custody. The invariant
> is in [AGENTS.md §2](../AGENTS.md).

### Code layout

| Path | What it is |
|---|---|
| `crates/demeteo-hub` | the axum server — storage, `KeyService`, `/healthz` plus static serving, passkey auth (registration, sign-in, sessions, endorsement and revocation), pairing, the instance socket, the session API and attachments **built**; unpairing **not built** (§10). It depends on `crates/demeteo-hub-protocol` |
| `crates/demeteo-hub/src/pairing/` | pairing (§4) — **built**. The decisions are synchronous functions that take the time as an argument: `authorize.rs` (the `/authorize` rules and the loopback redirect), `pkce.rs`, `jwk.rs` (thumbprint and fingerprint), `dpop.rs` (proof verification) and `grant.rs` (the redeem, refresh and access verdicts, and the response bodies). `device.rs` is a runner's pending device codes and their verdicts, `pending.rs` holds consents, codes, device codes and seen proofs in bounded process memory, `signing_key.rs` reads the request-signing public key through `KeyService`, `access.rs` is the admission of a DPoP-bound access token that the socket and the attachment fetch share, and `handlers/` holds the routes. The rows are written by `store/instances.rs`, `store/device_keys.rs` and `store/tokens.rs` |
| `crates/demeteo-core/src/domain/hub_pairing/` | the desktop's pairing policy — **built**. The rules both sides must spell alike are mirrored here, because `demeteo-core` does not depend on `demeteo-hub`: the origin and the URLs derived from it, the callback verdict, PKCE, the thumbprint and fingerprint, the DPoP signing input, and the mirror of the pairing response. `src-tauri/tests/hub_desktop_pairing.rs` runs the desktop against the Hub's own handlers, which keeps the two copies from drifting |
| `crates/demeteo-core/src/domain/hub_socket/` | the desktop socket's policy — **built**, and synchronous: `src-tauri/tests/hub_layout.rs` fails on an `async fn` anywhere in it. `projection.rs` is the allowlist a `run_events` row passes to become a protocol `RunEvent`, `mod.rs` the `Hello` and whether events are sent at all, and `verdict.rs` what follows a connection's or a refresh's end, the backoff, and the numbers the session runs on |
| `crates/demeteo-core/src/ports/hub.rs` | the ports pairing, the socket and the requests reach the world through: the keyring (`HubSecretStore`), SQLite (`HubPinRepository`, `HubRequestRepository`, and the policy's `HubPolicyReader` and `HubPolicyWriter`, kept apart so the handler can be given the first alone), the Hub's `/token` for both grants (`HubTokenEndpoint`, `HubRefreshEndpoint`), the socket (`HubSocketDialer`, `HubSocketConnection`), the attachment fetch (`HubAttachmentFetcher`), the trusted passkey set and its counters (`HubPasskeyTrust`, over the `sign_count` column migration `V66__hub_passkey_sign_count.sql` adds, with its SQL beside the pins' in `adapters/database/repos/hub.rs`), the request handler the socket hands frames to (`HubRequestHandler`) and a browser — **built**. Only the keyring port carries a secret |
| `crates/demeteo-core/src/adapters/hub/` | the desktop's pairing I/O — **built**. `HubPairing` in `mod.rs` orders the steps and owns the refresh lock and the socket's lifetime; `device_key.rs`, `loopback.rs`, `token_client.rs` and `keyring_store.rs` are the adapters, and `attachments.rs` fetches an attachment once and checks its size and SHA-256 (§6.3). It is composed only under `ExecutionMode::Router`, and `src-tauri/src/commands/hub.rs` exposes it |
| `crates/demeteo-core/src/domain/hub_request/` | the desktop's request policy (§6.1, §7) — **built**, and synchronous. `mod.rs` is the scopes in force, `precheck` (signature, scope, instance, expiry) and `decide` (replay, then spend), split around the consumption of the id; `ceiling.rs` the two ceilings, `verify.rs` the Ed25519 check against the pinned key, `log.rs` the activity-log row, and three for §8: `cose.rs` the parse of a canonical `COSE_Key`, `assertion.rs` the WebAuthn assertion check and the counter rule, and `passkeys.rs` the three challenge digests and the gate, endorsement and revocation verdicts |
| `crates/demeteo-core/src/application/hub_requests.rs` | the request handler — **built**. It parses, loads the pins, the policy, the committed total and the trusted passkeys, records the id, and acts through the desktop's own launch, cancel, assignment and `gate_decide` calls. `hub_requests/start.rs` holds a start once its id is spent, and `hub_requests/trust.rs` the I/O of the three kinds a passkey signs: loading the set and a gate's step, the counter or set write, and the gate decision. `composition/mod.rs` binds it to `HubPairing`. The SQL is `adapters/database/repos/hub_requests.rs` |
| `crates/demeteo-core/src/adapters/hub/socket/` | the desktop socket's I/O — **built**. `mod.rs` is the supervisor (refresh, proof, dial, session, verdict, backoff), `session.rs` one connection from its `Hello` to its end, `wake.rs` the `RunEventsPort` decorator that wakes a session on every append, and `ws_transport.rs` the `HubSocketDialer` over `tokio-tungstenite` and rustls, the only file that names the WebSocket crate. `ws_transport.rs` exists only with `demeteo-core`'s default `hub-socket` feature, which `demeteo-runner` turns on by name without `keyring` (`src-tauri/tests/hub_layout.rs` gates both) |
| `crates/demeteo-hub/src/socket/` | the instance socket (§5) — **built**. `handler.rs` holds the `GET /socket` upgrade and its admission (`admitted`, which names the socket's URL to `pairing/access.rs`), `session.rs` the loop one connection runs, `frame.rs` the rule that tells inbound frames apart, `cursor.rs` how the cursor moves, `connections.rs` the registry of live sessions in process memory, and `requests.rs` the queue: `enqueued`, delivery and outcomes. `port.rs` is the two ports the session is written against, `Transport` and `SocketStore`; `postgres.rs` is the store over one `(UserId, InstanceId)`, writing through `store/instances.rs`, `store/run_mirror.rs`, `store/snapshots.rs` and `store/requests.rs`. The module rustdoc of [`socket/mod.rs`](../crates/demeteo-hub/src/socket/mod.rs) says what an open socket proves |
| `crates/demeteo-hub/src/api/` | the session API (§6.4) — **built**. Each route is a thin handler over a use-case that takes the tenant, `now` and an `ApiStore` (`port.rs`), so it runs without Postgres: `fleet.rs`, `runs.rs`, `dispatch.rs`, `gates.rs`, `request_status.rs` and `attachments.rs`, the upload. The decisions are synchronous: `run_state.rs` folds the run mirror into a Feature's status, steps, tickets and pending gates, `scope.rs` is the read-scope verdict, and `gates.rs` holds the structural assertion check. `postgres.rs` is the store over the `store/` reads, and `testing.rs` the multi-tenant in-memory double, `MemoryFleet` |
| `crates/demeteo-hub/src/attachments/` | attachments (§6.3) — **built**. `envelope.rs` is the sealing and the decisions, synchronous and without I/O: the size verdict, the expiry and which ids a dispatch may bind. `port.rs` is the `AttachmentStore` port, `postgres.rs` the store over `store/attachments.rs`, `fetch.rs` the fetch-once use-case and its route, and `sweep.rs` the TTL sweep `main.rs` spawns. The upload is a session route, so it is `api/attachments.rs` |
| `deploy/hub` | the Compose bundle, Dockerfile and operator README — **built** |
| `crates/demeteo-hub-protocol` | **serde-only** wire types shared by the desktop client and the Hub — **built**; `demeteo-hub` and `demeteo-core` depend on it, and it depends on neither |
| `hub-web/` | a second Vite entry in this repo; it reuses `src/components/canvas` and the `src/App.css` tokens — **sign-in, passkey management and the two pairing approval pages built** (SignUp, SignIn, Passkeys, ApprovePairing at `#/pair/<segment>`, ApproveDevice at `#/device`, and a session gate in front of every route but the first two); Fleet reads `GET /v1/fleet` with a Needs-you lane of pending gates, Instance reads `GET /v1/instances/{instance_id}`, AddInstance explains desktop and runner pairing, and Runs, Run and Dispatch are placeholder routes with no data wiring yet; no view renders the canvas yet. `npm run build:hub-web` writes a static bundle to `hub-web/dist`, and `demeteo-hub` serves the directory its configuration names |

**Rejected: the Hub linking `demeteo-core` in v1.** `demeteo-core` brings `libssh2`,
git plumbing and `rusqlite`. A hosted container would ship all three for code that
v1 never calls, and the Hub's attack surface would be the desktop's. Keeping the
wire types in a crate of their own means the desktop client and the Hub share a
spelling without sharing an implementation. It also keeps the protocol crate free
of I/O, so a request's verification can be tested with no port double — the shape
[AGENTS.md §3](../AGENTS.md) asks of a policy decision.

`hub-web/` reuses the canvas components and the tokens, so the Hub's workflow
graph is the desktop's: [AGENTS.md §4](../AGENTS.md) applies, token values come
from `src/App.css`, and nothing is hard-coded.

Building any of this adds Cargo and npm dependencies (`axum`, a Postgres client, a
WebAuthn verifier). Each one is a **Gate item** in
[AGENTS.md §6](../AGENTS.md): the ticket stops and asks.

---

## 10. What is not built

§2 has the state of every part. Of pairing (§4) and the socket (§5), the Hub's
half, the approval page and the desktop's device key, loopback listener,
`/authorize` request, token redemption and refresh, fingerprint and socket client
are built, and so are the desktop's request verification and local ceilings
(§6.1, §7) and its verification of gate decisions and passkey changes against
the pinned set (§8), and the Settings › Hub tab (§2). Three things are not:

- **What the desktop's socket does not send.** It sends no `ticket_*` event,
  because no desktop `run_events` row records a ticket's lifecycle, so the
  Hub's ticket counts read zero; and it relays local runs only (§5 *Events and
  the cursor*). §11 *The desktop socket's carried defaults* has what it does
  instead.
- **Two gaps in Hub gate decisions and passkey changes**, each recorded so it
  is not mistaken for a decision:
  - *A gate of a run a `demeteo-runner` owns.* The desktop's `gate_decide`
    refuses a run it mirrors from a runner, which also has no local gate row,
    so a Hub `gate_decision` for one is answered `runner_owned` before its
    assertion is checked (§8). The desktop's own view decides those gates
    through `remote_decide_gate` instead; a Hub cancel already goes to whoever
    owns the run, found by the same remote-run mirror lookup. Closing it needs
    a pending check against the runner, and the conformance suites of
    [AGENTS.md §7](../AGENTS.md) to prove the two paths alike.
  - *No Hub producer for the passkey kinds.* The desktop verifies and applies
    a `passkey_revocation` bound to its own id (§8), and consumes and refuses
    every `passkey_endorsement` as `unknown_kind` without reading its assertion
    (§11 item 20). No Hub route queues either for an instance, and the Hub's
    own records bind `"*"` (§11 item 9). Until one does, only tests reach the
    revocation path.
- **Unpairing and revoking an instance on the Hub.** No route does either, and
  nothing writes `device_keys.revoked_at`. The desktop's disconnect is local
  (§4 *Revocation*). Until a route exists a pairing ends by reuse detection, by
  its refresh token expiring, or by being replaced (§4).
The session API (§6.4) queues requests, and a paired desktop answers them,
fetching a `start_feature`'s attachments (§6.3) before it answers. Two routes take an access
token, the socket (§5) and the attachment fetch (§6.3), and both are admitted
by [`pairing/access.rs`](../crates/demeteo-hub/src/pairing/access.rs) through
`access_verdict` in [`grant.rs`](../crates/demeteo-hub/src/pairing/grant.rs).

**Runner dispatch is decided and not built.** Decisions
[64](DECISIONS.md#64--hub-instances-detail) and
[71](DECISIONS.md#71--runner-git-credentials-detail) settle its shape: a
`demeteo-runner` host pairs and holds its own outbound socket, verifying,
enforcing and reporting as a desktop does; the Hub builds the `RunSpec` from the
last snapshot a paired desktop reported (§5); and the Hub mints each run's git
token from a GitHub App key in OpenBao, delivered in the signed request and
refreshed over the socket. Until it is built, no request kind in §6 targets a
runner.

Two things about it are easy to get wrong:

- [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) says scoped-token minting is not
  implemented and standing PAT injection is what ships. That is still true for
  desktop-submitted runs, and stays true after runner dispatch lands: only a run
  the Hub starts gets a minted token.
- **The socket is not an execution transport.** A Hub-started run executes
  through the runner's own local `ExecutionPort`, and a Hub request must land in
  the same application functions a desktop's control RPC does (decision 64).

A runner pairs by device code (`demeteo-runner hub pair <url>`), approved on the
Hub with a passkey; both halves of that are built (§4, §2). Its scopes and
ceilings are set on the runner by its CLI or by a desktop over SSH
(decision 64); a desktop's machine settings do not call its `hub_*` methods
yet. GitLab runs get their tokens from a
dedicated bot user's OAuth authorization (decision 71).
[`hub-design/README.md`](hub-design/README.md) lists the mockup's version of the
pairing.

---

## 11. What is not decided

Open means unresolved. Nothing below is decided, and a build ticket that needs one
should raise it, not pick. Where an item says a **default is built**, the Hub
does that today because the socket could not be built without an answer; no
decision stands behind it, a human may overrule it, and until one does it is
what a desktop client implements.

1. **Gate decisions other than `approve` and `cancel`.** `redirect` carries
   free-text feedback the challenge does not bind (§8). Whether the Hub offers it,
   and how the feedback would be bound, is open. Until then an instance refuses it.
2. **What a Gate's review pane may show on the Hub.** Transcripts, diffs and file
   contents never leave an instance (§3), so a Gate reviewed on the Hub is reviewed
   without them. Which gate summary text crosses, written by whom, is open.
3. *Decided by the session API build, kept so the numbering holds.*
   `cancel_feature` needs `spend`: §6.2 assigned it, `required_scope` in
   [`socket/requests.rs`](../crates/demeteo-hub/src/socket/requests.rs) encodes
   it, and the cancel route of §6.4 is refused without it.
4. *Decided by the request-verification build, kept so the numbering holds.*
   The rolling 24-hour total counts the caps committed at start, not cost
   incurred, in integer cents; an unset ceiling and an absent cap each refuse;
   and `set_step_assignment` counts against neither ceiling. §7 *Local ceilings*
   has the rule. **Still open:** the per-run ceiling bounds a start's declared
   cap, which is a per-turn budget, so nothing bounds what a Hub-started run
   actually spends.
5. **A cursor gap.** If `run_events` ever gains a GC (none today), how the instance
   tells the Hub its cursor is older than what it holds (§5).
6. **Renewing the access token on an open socket**, since it lives 15 minutes and
   the socket does not, and what happens to the socket when the pairing is revoked.
   **Default built:** a token is not renewed on a socket. The Hub closes the
   socket with `4001` at the expiry of the token it was admitted with, and the
   instance refreshes and reconnects — one reconnect per 15 minutes per
   instance (§5). A socket left open past that expiry would outlive §4's
   revocation bound; this keeps it, so a revoked pairing loses its socket at
   that expiry at the latest and cannot open another. Renewing in place, and
   closing a revoked pairing's socket sooner, stay open.
7. **Hub-key rotation.** The request-signing key is pinned at pairing (§4); how a
   rotated key reaches an instance is not designed.
8. **A protocol version gate.** `hello` carries the version; what an instance and
   the Hub do on a mismatch is not decided. [Decision 55](DECISIONS.md#1-the-locked-decisions)
   is the runner's answer and is not assumed to carry over.
   **Default built, the Hub's half only:** an inbound frame whose `v` is not
   the Hub's protocol version closes the socket with `1008` before anything
   is written, a `Hello` included (§5). There is no negotiation and no range
   of versions. What an instance does about a frame of another version from
   the Hub is still open.
   **A new variant an instance sends is a wire change**, a new `RefusalReason`
   included, though `v` stays the same. `RefusalReason` has no
   `#[serde(other)]`, so a Hub built before the variant cannot parse a result
   that carries it. Its session ends on the frame error and closes the socket
   with `1008`. The desktop reads `1008` as `StopReason::ProtocolViolation` and
   stops reconnecting until an app restart or a re-pair. After that, the
   redelivered request is answered `replayed`, because its id is already spent,
   so the Hub never learns the real reason. **Until a version gate exists, the
   Hub ships first:** upgrade it before any desktop that can send the new
   value. `runner_owned` is the first reason added since v1. An honest Hub
   never draws it, because the socket relays local runs only (§5), so no Hub
   view offers a gate on a run a `demeteo-runner` owns.
9. **Defaults carried, not measured:** ES256 for the device key, EdDSA for the
   envelope, 128 bits for a request id, the field lists of the two passkey
   challenges, and an upper bound on `expires_at` an instance will accept.
   Pairing carries more, listed under *Pairing's carried defaults* below, the
   socket under *The socket's carried defaults*, and attachments under *The
   attachments' carried defaults*. One
   more is already in stored data: the Hub's own endorsement and revocation
   assertions bind `instance_id = "*"`, a fresh `request_id` and a 300-second
   `expires_at`, all stored with the record (§8). How an instance accepts such
   a record — relayed as it stands, or replaced by a fresh assertion bound to
   that instance — is not decided. **Default built, the desktop's half only:**
   an instance recomputes every passkey challenge with its own id, so it
   accepts a fresh instance-bound revocation and refuses a record relayed as
   it stands, whose challenge is another. It refuses every endorsement,
   however bound (item 20). No Hub route sends either yet (§10).
10. *Decided by the desktop pairing build, kept so the numbering holds.* The
    desktop-side client sits in `demeteo-core`: its decisions in
    `domain/hub_pairing/`, its ports in `ports/hub.rs`, its I/O in
    `adapters/hub/`, and `commands/hub.rs` only delegates (§9 *Code layout*).
    Its loopback callback binds `127.0.0.1:0`, takes the port the OS assigns,
    and serves one random path; nothing configures either.
11. **Whether a step's `command` and prompt templates may cross at all.** They
    travel inside a Workflow snapshot's documents, and decision 65 needs them:
    a `RunSpec` cannot be built without the command and the prompt (§10). But
    both are user-authored text that may embed a local path or a credential
    (`curl -H "Authorization: Bearer …"`), which no field-level strip can
    detect. Sending them, redacting them, or sending a projection without them
    — the last re-decides decision 65 — is open. Until it is decided the
    desktop sends them, scrubbed (§11 *The desktop socket's carried defaults*).
    The snapshot builder's settled obligations, `cwd` and `remote_url`
    userinfo, are in §5.
12. **Approving a pairing on a session alone.** The consent routes (§4) take a
    signed-in session, guarded by `Origin` and the CSRF header, and no fresh
    assertion. §8 refuses to let a session move *passkey* trust, and part of
    that reason applies here: a hijacked session can add an instance, which
    then reports events and collects queued requests as the tenant's. It cannot
    decide a gate (§8). Whether an approval should carry an assertion is open.
13. **`client_id`.** RFC 6749 names it. The Hub registers no client and
    `/authorize` defines no such parameter, so one that is sent is ignored like
    any other unknown parameter. Whether desktops are ever registered, and what
    an id would then identify, is open.
14. **Gaps pairing leaves, known and not closed:**
    - *The sign-up detour.* Sign-in keeps the `#/pair/<segment>` address and
      lands back on the approval page. The link from sign-in to sign-up replaces
      it, so a user with no passkey yet signs up, lands on Fleet, and starts the
      pairing again from the desktop.
    - *No framing protection.* No response sets `X-Frame-Options` or a
      `Content-Security-Policy`, so the approval page can be framed by another
      site.
    - *Pending state in process memory.* Consents, authorization codes and the
      DPoP replay window live in the Hub's memory: a restart drops every pairing
      in flight, and a second replica would not see the first's. `Ceremonies`
      accepts the same cost for passkey ceremonies.
    - *The connection registry in process memory.* Which instances hold a
      socket, and the signal that wakes or displaces one, live in the same
      memory at the same cost, so the Hub is **single-replica** for the socket
      too. A request queued on one replica wakes no session held by another
      and waits until that instance reconnects. Displacement does not cross
      replicas: each can hold a connection of the same instance, and the
      "one connection per instance" of §5 holds per process. A restart drops
      every socket. A clean stop asks each session to end and waits, up to
      `SHUTDOWN_DRAIN_SECS` (five seconds, under the container's ten-second
      stop grace), for it to record its departure and send `1001`. Only a
      kill, a crash or a session that outlasts that bound leaves its instance
      reading as present until its next `hello`: nothing clears presence at
      startup.
    - *A code burned by a sealed key service.* The code is taken before the
      request-signing key is read, so with OpenBao sealed `/token` answers
      `503 key_service_unavailable` and the code is spent. The user repeats the
      pairing after the unseal.
15. **A second writer of `granted_scopes`.** Pairing writes what the user
    approved; `hello` overwrites it with what the toggles allow now (§7).
    **Default built:** `hello` wins and the approved set is not kept, so the
    Hub cannot show "approved at pairing" beside "on now". Keeping the
    approval as a ceiling takes a second column, and nothing would enforce
    with it. Whether the Hub should remember it is open.
16. *Decided by the session API build, kept so the numbering holds.* A
    session may queue a `gate_decision`, and only one carrying an assertion
    over a challenge the Hub minted for that session (§6.4). That is §8's
    rule, not an exception to it: the session is necessary and not
    sufficient, the Hub checks the assertion's shape only, and the instance
    verifies it against its pinned set. Every queueing route is under `/v1`
    behind the session and CSRF guard.
17. **A changed `install_id` behind the same device key** — a desktop whose
    database was lost under a keyring that survived. Its run log starts again
    at offset 1, under a cursor the Hub holds from the earlier installation.
    **Default built:** the Hub stores the new id, logs the change by instance
    id, and **keeps the cursor**; the new installation's events arrive beneath
    it, and one that meets an old row under the same Feature and offset is
    dropped as a copy of it. Resetting the cursor, or setting the earlier
    installation's rows aside, is a product decision nobody has taken. The
    `install_id` is a label either way and never chooses the instance (§5).
18. **Snapshot granularity.** `snapshots` is keyed to hold one row per kind
    of snapshot — `projects`, `workflows` — each replaced on its own. The
    protocol's `Snapshot` is one struct carrying both, with no kind on it.
    **Default built:** one row per instance, replaced wholesale (§5).
    Splitting it — so that a change to one Project does not resend every
    Workflow — is a change to the protocol crate first.
19. **The plaintext digest in a stored envelope.** A `start_feature` signs
    each attachment's content digest, and the stored envelope keeps it for as
    long as the request row lives, which is for good (§9).
    **Default built:** kept, so the stored signature still verifies. Redacting
    it once the request is answered or expired, deleting old request rows, or
    signing something other than the content digest are each open.
20. **A key endorsed through a substituted ceremony.** A compromised Hub can
    spend one ceremony the user takes for something else, a sign-in included,
    on an assertion by a trusted key A over an `endorsement_challenge` for a key
    X it holds. Every check of §8 passes, so a desktop that applied it would
    trust X with no prompt, and X could decide every later gate. X could also
    revoke the user's keys down to {X}: a revocation by X of A is accepted while
    {A, X} are trusted, after which the last-key guard protects X, the user's
    own passkey is refused `credential_not_pinned`, and only a re-pair
    recovers. The activity-log row would be the desktop's only record, and no
    view shows it (§2).
    **Default built: refused.** The desktop consumes every
    `passkey_endorsement` and answers it `unknown_kind` without reading its
    assertion or loading the pinned set. `unknown_kind` already means *this
    build does not act on this kind*, and a new `RefusalReason` would close an
    older Hub's socket (item 8). Nothing legitimate is lost: no Hub route
    queues one for an instance (§10). It stays refused until both exist: a Hub
    producer of instance-bound endorsements (item 9), and a desktop-side
    confirmation of each endorsement before its key is trusted, or a view of
    the trusted set in Settings › Hub (§2). The confirmation reopens part of
    what §8 rejects, accepting each new passkey by hand on every desktop.
    Accepting the risk in writing instead is the other way to lift it, and no
    one has.
    **What remains:** a substituted ceremony can still spend that one ceremony
    on one gate decision the user did not mean, or on revoking one of the
    user's other pinned keys — never the last, and never adding one.

### Pairing's carried defaults

One lifetime in §4 is decided: the access token's 15 minutes. Every other number
pairing runs on is a **carried default** — chosen so the routes could be built,
measured against nothing, and a human may override any of them. Each is a
constant in [`pairing/mod.rs`](../crates/demeteo-hub/src/pairing/mod.rs).

| Default | Value | What moving it changes |
|---|---|---|
| Refresh-token lifetime | 30 days | a desktop offline for longer must pair again |
| Authorization-code lifetime | 60 seconds | how long the desktop has to redeem after the user approves |
| Consent lifetime | 300 seconds | how long the approval page may sit open before its answer is `pairing_expired` |
| DPoP proof skew | 60 seconds either side of the Hub's clock | how far a desktop's clock may drift, and how long a proof is remembered against replay |
| Pending consents per user | 8 | a ninth concurrent request is `503 too_many_pairings` |
| Pending consents, and pending codes | 16,384 each | the Hub-wide bound behind the same answer |
| Remembered DPoP proofs | 65,536 | a full window answers `503 too_many_requests` and refuses, never forgets |
| Device-code lifetime, and poll interval | 600 seconds, 5 seconds | how long a runner's code may wait for a human, and how often it may ask; both are what `POST /device_authorization` tells the runner |
| Pending device codes | 4,096, Hub-wide | a start beyond it is `503 too_many_pairings`. There is no per-caller bound: a start needs no session and a device key costs nothing |
| `user_code` | 8 of 20 consonants, about 34.6 bits | with the bounds beside it, one tenant's window of guesses finds one of 4,096 live codes with odds near 1 in 600,000 |
| Device-code lookup misses | 10 per tenant per 600 seconds | the eleventh lookup in a window is `429 too_many_lookups`, the right code included |

Carried with them, and stated in §4: ES256 as the only device-key algorithm, the
fingerprint format and its 96 bits, the Hub minting `instance_id`, a second
pairing of the same device key replacing the first, and scopes that only narrow
at approval with none of them mandatory.

### The socket's carried defaults

The same holds for every number the socket runs on: chosen so it could be
built, measured against nothing. Each is a constant in
[`socket/mod.rs`](../crates/demeteo-hub/src/socket/mod.rs).

| Default | Value | What moving it changes |
|---|---|---|
| Ping interval | 30 seconds | how often an idle connection carries traffic, and what that costs a desktop that would otherwise sleep |
| Silence deadline | 90 seconds | how long an instance whose connection died without a close goes on reading as present. It must stay above the ping interval |
| Request lifetime | 600 seconds | how long a request for an offline instance stays deliverable. It is signed into the envelope, so a change reaches no request already queued |
| Request id | 128 random bits | — |
| Connected instances per `hub` process | 16,384 | the bound behind `503 too_many_connections` |
| Largest message, and largest frame | 4 MiB | the largest snapshot an instance can deliver, and what one connection can make the Hub buffer. A message over it is not read: the session ends as if the connection had failed, with no close code |

Carried with them, and stated in §5 and §6.1: frames with no tag, a proof's
`htu` in its `https` spelling, the close codes, signing the pre-image with no
digest in between, and signing once at enqueue. The two that answer an open
question — the `4001` close and the version gate — are items 6 and 8 above.

### The desktop socket's carried defaults

The desktop's client (§9 *Code layout*) was built ahead of any Settings view,
and its snapshots ahead of item 11, so it carries an answer for each gap. None
is decided; each is what the desktop does today.

- **A deferred request is left unanswered and the socket stays open.** A
  request the desktop cannot judge for a storage failure gets no
  `RequestResult` and its id is not spent (§6.1 *On the desktop*). The Hub
  keeps the first outcome it records (§5 *Queued requests*), so a `refused`
  sent for a passing failure would stand for good; unanswered, the request is
  delivered again on the next connection until it expires. A session with no
  handler bound leaves every request unanswered the same way.
- **An equal non-zero signature counter is refused** as a possible clone, by
  the counter rule of §8 *What the desktop verifies*. Accepting a count that
  merely does not go backwards would let it through; which of the two a gate
  needs was not decided.
- **A key's earlier decision can be refused for a later one.** Each request
  is handled on a task of its own, so two the Hub delivers in queue order can
  take the hold of §6.1 *On the desktop* in either order, and the counter is
  compared with whichever count was stored last. A key that keeps a count,
  such as a hardware security key or Windows Hello, approves gate G1 at 10 and
  G2 at 11 while the desktop is offline: if G2 takes the hold first it is
  accepted, and G1 is refused `assertion_invalid` with its id spent. A
  decision deferred for a storage failure is refused the same way when its
  redelivery arrives after a later one from that key was accepted. Synced
  passkeys, which report 0 every time, are unaffected. The recourse is to
  decide that gate again, a new ceremony with a new count. Handling the kinds a
  passkey signs in delivery order was not built: a deferred request comes back
  only on a later connection, by when one queued behind it may have been
  accepted, so order cannot reach that case; and the session reads no further
  into a frame than its `request_id`, so ordering there would change how it
  runs every kind.
- **The policy, the activity log and the consumed ids outlive the pairing.**
  `hub_disconnect` clears the pins and the keyring entries and nothing else
  (§7 *Local ceilings*).
- **A step's `command` and prompt templates cross, scrubbed.** Item 11 is open;
  meanwhile every string left in a Workflow document goes through
  [`secret_scrub.rs`](../crates/demeteo-core/src/shared/secret_scrub.rs), which
  masks `user:pass@` URLs and known token shapes. A path in that text crosses.
- **A rebuild runs 2 seconds after the first write that wakes it**
  (`SNAPSHOT_DEBOUNCE_SECS`). The window is fixed, not trailing: writes inside
  it do not move it, so continuous edits cannot hold a snapshot back.
- **Each Workflow is sent at its latest version only.** Every save appends a
  version, and sending them all grows towards the 4 MiB frame.
- **A provider edit does not wake a rebuild.** Adding or deleting a provider
  changes the `remote_url`s derived from it, which stay stale until the next
  Project or Workflow write or the next connection.
- **A Project reports one `remote_url`**, its first Repository's.
  `ProjectSnapshot` has one field, and reporting every Repository is a
  protocol change.
- **No `ticket_*` kind is sent.** No desktop `run_events` row records a ticket's
  lifecycle, so [`projection.rs`](../crates/demeteo-core/src/domain/hub_socket/projection.rs)
  has nothing to map onto one.
- **`Hello` reports the pinned scopes as granted, and events and snapshots go
  only when `read` is pinned and its toggle is on.** It does not narrow them by the toggles of §7, so the Hub's
  copy can list a scope whose toggle is off; a request for it is queued and comes
  back `refused: scope_off`. With `read` off, the Hub's copy still lists
  `read` while the socket sends nothing past its `hello`.
- **A stop is shown, and does not unpair.** A verdict that stops the socket logs its
  reason with `tracing::warn!`, and Settings › Hub shows it from
  `hub_connection`'s `stop_reason`. `HubStatus` is unchanged, so a desktop
  whose refresh family was revoked still reads as paired until the user pairs
  again. After `invalid_grant` the refresh token stays in the keyring. A stop
  holds until `hub_pair` or the next app start.

| Stop reason | What ends there |
|---|---|
| `Displaced` | a `4002` close: redialling would displace the connection that displaced this one |
| `NeedsRepair` | a refresh refused with `invalid_grant` |
| `RefreshUncertain` | a refresh sent with no answer read: the Hub may have rotated the token, and presenting the old one again would revoke the family |
| `ClockSkew` | a refresh whose proof the Hub refused for its time |
| `ProtocolViolation` | a `1008` close |
| `Unauthorized` | an upgrade `401` to an access token refreshed for that very attempt |
| `Refused` | any other refresh refusal, an upgrade refusal that is neither `401` nor `429` nor `5xx`, or an origin or proof the desktop cannot build |

The numbers are constants in
[`verdict.rs`](../crates/demeteo-core/src/domain/hub_socket/verdict.rs):
a backoff of `2^attempt` seconds capped at 300 and scaled by a jitter in
`[0.5, 1.0]`, reset by the first `CursorAck` of a session; 90 seconds of
silence before a connection is dropped, mirroring the Hub's deadline above;
128 rows read per page; and at most 256 events sent past the last ack.

### The attachments' carried defaults

The numbers attachments run on (§6.3) are carried defaults in the same sense.
The first three are configuration, with defaults in
[`attachments/mod.rs`](../crates/demeteo-hub/src/attachments/mod.rs); the fourth
is a constant there.

| Default | Value | What moving it changes |
|---|---|---|
| Largest upload, `DEMETEO_HUB_ATTACHMENT_MAX_BYTES` | 25 MiB | the largest file a Feature can be started with from the Hub, and what one upload or fetch makes the Hub buffer |
| Staged TTL, `DEMETEO_HUB_ATTACHMENT_TTL_SECS` | 3600 seconds | how long a file may wait for the dispatch that names it. It does not bound a bound attachment, whose expiry is its request's |
| Sweep interval, `DEMETEO_HUB_ATTACHMENT_SWEEP_SECS` | 60 seconds | how long a row may outlive its expiry before it is deleted |
| Attachments per dispatch, `MAX_ATTACHMENTS_PER_DISPATCH` | 10 | kept equal by hand to the desktop's own limit per Feature |

No per-tenant cap on staged uploads is built: a tenant can stage as many as it
uploads within one TTL.

---

## 12. Related

- [`DECISIONS.md`](DECISIONS.md) — decisions 62–69, the short form of this document
- [`hub-design/README.md`](hub-design/README.md) — the mockup, and where it is superseded
- [`MCP_INTEGRATION.md`](MCP_INTEGRATION.md) — the other external surface; §5 for the OAuth shape this one differs from, §8 for decision 50
- [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md) — §17, the web-companion bullet this reverses
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the hexagon, and the crates this adds
- [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) — the contract a future runner dispatch must not fork
- [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) — runners, and the credential state §10 refers to
- [`../AGENTS.md`](../AGENTS.md) — §2 invariants, §6 Gate policy
