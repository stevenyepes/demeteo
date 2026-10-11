# Demeteo Hub — Design and Decisions

> **Scope:** how a browser control plane reaches many Demeteo instances — who
> trusts whom, how an instance pairs, what crosses the wire, what the Hub may ask
> an instance to do, and what never leaves an instance. **Built:** nothing. The
> design is decided (2026-10-02) and no part of it is implemented; §2 says so per
> part. [`hub-design/README.md`](hub-design/README.md) owns the mockup,
> [`ARCHITECTURE.md`](ARCHITECTURE.md) owns the hexagon the desktop-side client
> will sit in, [`MCP_INTEGRATION.md`](MCP_INTEGRATION.md) owns the other
> external surface into a desktop — a different surface, with a different trust
> argument (§8) — and [AGENTS.md §2](../AGENTS.md) owns the invariants this
> document must not weaken.
>
> The wire field names in §5 and §6 are **illustrative** unless a section says a
> name is fixed by decision. The decisions fix the behaviour; the
> `demeteo-hub-protocol` crate will fix the spelling, and until it exists a name
> here is a proposal, not a contract.

---

## 1. Summary

Decisions [56–63](DECISIONS.md#1-the-locked-decisions) are the short form of what
follows. Each row here has a section below that states the rejected alternative in
full.

| # | Decision | Rejected | Why |
|---|----------|----------|-----|
| 56 | A browser control plane (the Hub) is in scope; reverses [`OPEN_QUESTIONS.md` §17](OPEN_QUESTIONS.md#17-other-captured-items) | keeping a web companion out of scope | one view over a fleet, and a gate decided away from the machine, are needs the desktop cannot meet |
| 57 | Hosted, multi-tenant; a tenant is one person with one fleet | self-hosted single-tenant; teams in v1 | the user wants a hosted service; the passkey endorsement chain has no notion of membership |
| 58 | Desktops pair and hold one outbound `wss`; the Hub never dials a desktop; runners are not paired | the runner dialing out; the Hub storing PATs or long-lived SSH keys | a Hub token at rest on a host with no keyring, plus a second transport; standing credentials for every tenant — see [decision 58](DECISIONS.md#58--hub-instances-detail) |
| 59 | The Hub builds future runner `RunSpec`s from desktop-reported snapshots with local paths stripped | Hub-side authoring of Projects and Workflows; Workflows read from repo files | a second editor plus drift; a new concept |
| 60 | A Gate may be decided from the Hub by a WebAuthn assertion the desktop verifies against pinned passkeys | a session plus a step-up for some gates; read-only gates; manual per-desktop passkey acceptance | a compromised Hub could approve ordinary gates; see §8 |
| 61 | Scopes `read` / `spend` / `gates`, toggled and enforced on the instance; local per-run and rolling-24-hour ceilings | a `logs` scope; MCP's `configure`; toggle-only scopes | transcripts and diffs never leave an instance; a Hub-supplied cap is a cap the Hub can raise — see §7 |
| 62 | Postgres plus OpenBao (Transit) as Docker Compose: `hub`, `postgres`, `openbao`, `caddy` | HashiCorp Vault; Cosmian/Eviden KMS | BSL is not open source; source-available licensing — see §9 |
| 63 | `demeteo-hub`, `demeteo-hub-protocol` and `hub-web/` | the Hub linking `demeteo-core` in v1 | it would ship `libssh2`, git plumbing and `rusqlite` in a hosted container for code v1 never calls |

---

## 2. Status of the parts

| Part | State |
|---|---|
| `crates/demeteo-hub` (axum server) | **not built** — the crate does not exist |
| `crates/demeteo-hub-protocol` (wire types) | **not built** — the crate does not exist |
| `hub-web/` (second Vite entry) | **app shell only** — placeholder routes (Fleet, Instance, Runs, Run, Dispatch, AddInstance) with no data wiring yet. `npm run build:hub-web` writes a static bundle to `hub-web/dist`; `demeteo-hub` will serve the directory its configuration names |
| The Docker Compose bundle (`hub`, `postgres`, `openbao`, `caddy`) | **not built** |
| Desktop side: device key, pairing, Settings › Hub tab, socket client | **not built** |
| Desktop side: passkey pin set, request verification, local ceilings | **not built** |
| An instance-wide read over `run_events` | **not built** — the read surface is per Feature today (§5) |
| Runner dispatch from the Hub | **not built, and not part of this ticket set** — see §10 |

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
| **Attachments**, briefly | the one piece of user-authored content on the Hub: a file the user attached in the browser to a Feature. Encrypted at rest, deleted after the instance fetches it — §6.3 |
| Its own server secrets | in OpenBao, or encrypted under Transit — §9 |

### What a compromised Hub can and cannot do

| Can | Cannot |
|---|---|
| read what an instance reported: project and workflow names, step status, cost | read a key, a transcript, a diff or a path — none was sent |
| withhold, delay or reorder requests | forge a request: the envelope is signed and the instance verifies it against the key it pinned (§6) |
| replay a request | replay one that works: ids are single-use and expire (§6) |
| queue a `start_feature` for an instance that has `spend` on | exceed the instance's local ceilings (§7) |
| relay a gate decision a human made | **make** one: no token approves; the assertion is verified on the desktop (§8) |
| — | turn a scope on, or raise a ceiling |

> **A stolen device key and refresh token** let an attacker speak as the instance:
> report false events and collect queued requests, attachments included. That is
> why the refresh token rotates with reuse detection (§4), and why the device key
> is created on the instance and kept in the keyring: it never exists anywhere
> that can be copied from a hosted service.

---

## 4. Pairing

Pairing makes one **desktop** an instance of one tenant. A `demeteo-runner` host
does not pair (§10). It reuses the shape of the OAuth listener in
[MCP §5](MCP_INTEGRATION.md#5-authorization) — loopback, PKCE `S256`, a consent the
human signs — and differs from it on purpose.

> **Do not "align" this with MCP's token.** MCP's is a 30-day bearer with no
> refresh because the listener it protects is on the same machine. The Hub is
> remote and hosted, so the access token is **15 minutes**, **bound to a key**, and
> the refresh token rotates. A copy of an MCP token is a local problem; a copy of a
> Hub token is a tenant's.

<!-- EXAMPLE: new -->

```
instance            device key: create, keep in the OS keyring           (once)
instance ─browser─▶ GET  /authorize   PKCE S256, redirect http://127.0.0.1:<port>/…,
                                      dpop_jkt = thumbprint of the device key
Hub                 passkey sign-in; consent: key fingerprint + scopes
Hub ─redirect────▶ http://127.0.0.1:<port>/…?code=…&state=…
instance ────────▶ POST /token        code + code_verifier, DPoP proof
Hub ─────────────▶ access token (15 min, bound to the thumbprint)
                   refresh token (rotating, reuse-detected)
                   pairing response: request-signing key, RP ID, passkey keys
```

| Property | Value |
|---|---|
| Device key | created **on the instance**, private half in the OS keyring only; the Hub is sent the public half. Algorithm ES256 is the proposed default (§11) |
| Fingerprint | a short rendering of the key's RFC 7638 thumbprint, shown **on both** the desktop and the Hub's consent screen so the user can compare them |
| Redirect | `127.0.0.1` on an **ephemeral port** the instance opens for the one callback (RFC 8252); never a custom scheme, never `localhost` |
| PKCE | `S256`, mandatory (RFC 7636) |
| Authorization code | single-use, bound to the device key by `dpop_jkt` (RFC 9449), so a code intercepted on the loopback cannot be redeemed with another key |
| Sign-in | a **passkey**; the Hub has no password |
| Consent | the user approves the device-key fingerprint and the scopes (§7), and may approve fewer than were asked for |
| `/token` | a DPoP proof on the request (RFC 9449) |
| Access token | **15 minutes**, bound to the device-key thumbprint (`cnf.jkt`); presented with a fresh DPoP proof on every use |
| Refresh token | **rotating**: each use returns a new one and retires the old |
| Reuse detection | a retired refresh token presented again **revokes the whole pairing** — a stolen token and the real one cannot both survive |

### The pairing response

The `/token` response of the pairing grant carries three things the instance pins.
A refresh returns none of them.

| Field | Pinned as | Used for |
|---|---|---|
| The Hub's **request-signing public key** | the key every envelope is verified against (§6) | rejecting a request the Hub did not sign |
| The Hub's **WebAuthn RP ID** | the RP ID every assertion must be made for (§8) | rejecting an assertion made for another site |
| The user's **passkey public keys** | the initial pinned set (§8) | verifying a gate decision |

The pinned set is **trust on first pair**: the instance has no way to check these
three values at pairing except that they arrived over the TLS connection the user
chose and that the user approved the pairing on a screen they signed into. After
pairing, nothing the Hub says changes the passkey set on its own authority (§8).

### Revocation

Unpairing from the desktop, or revoking an instance on the Hub, retires the refresh
token family. The instance's local state — pinned keys, ceilings, toggles — is
dropped on unpair. Revocation takes effect at the next refresh, so at most one
access-token lifetime (15 minutes) after the Hub-side revoke.

---

## 5. The socket

**One outbound `wss` per instance.** The instance opens it; the Hub never dials a
desktop (decision 58). Events travel up, requests travel down, over the same
connection. The access token and a DPoP proof are presented on the upgrade.

<!-- EXAMPLE: new -->

```
instance → Hub   hello      install id, version, OS, arch, granted scopes
Hub → instance   ack        cursor the Hub has durably stored
instance → Hub   events     run events with offset > cursor, in offset order
Hub → instance   ack        the new cursor
instance → Hub   snapshot   Project and Workflow state, local paths stripped
Hub → instance   request    one signed envelope (§6); queued ones first
instance → Hub   result     request id; applied | refused | expired, and why
```

| Frame | Direction | Carries |
|---|---|---|
| `hello` | instance → Hub | install id, app version, OS, arch, **granted scopes** — what the instance's toggles allow *now*, not what pairing approved |
| `ack` | Hub → instance | the highest event offset the Hub has stored durably |
| `events` | instance → Hub | a batch of run events, each with its Feature and offset |
| `snapshot` | instance → Hub | the full current state of one kind (`projects`, `workflows`), replacing the last |
| `request` | Hub → instance | one signed envelope |
| `result` | instance → Hub | the outcome of one request id |

### Events and the cursor

Events come from the desktop's append-only `run_events` table
(`crates/demeteo-core/migrations/V22__run_events.sql`). Its `id` is
`INTEGER PRIMARY KEY AUTOINCREMENT`, so the **offset is table-global**: unique and
monotonic across every Feature on that desktop, not restarted per Feature.

**The cursor is per instance, not per Feature.** One high-water mark over the
table covers every Feature, which is what the Instance artboard's single offset
shows. A Feature's own history is still addressable because every event carries
its Feature. The existing read, `run_events_since(feature_id, from_offset)`
(`crates/demeteo-core/src/application/agent_surface/reads.rs`), is per Feature, so
the instance-wide read is new work (§2).

**Delivery is at-least-once and the Hub makes it idempotent.** An event is keyed
`(instance, feature, offset)`; the Hub drops a key it already holds. A dropped
connection costs a resend of whatever the last `ack` did not cover, never a gap
and never a duplicate row. The instance resumes from the Hub's `ack`, not from its
own memory of what it sent.

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
stripped** (decision 59). Each `snapshot` frame replaces the previous one of its
kind, so the Hub holds one current copy and nothing to merge. A snapshot is sent
after `hello` and whenever its content changes.

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
| Each step's `cwd`, in both Workflow documents | the whole field — it names a directory in the instance's tree |
| Any other local path in a Workflow document | the same |
| `remote_url` | userinfo (`https://user:token@host/…` → `https://host/…`); a Git token never leaves an instance (§3) |

A step's `command` and prompt templates are not on that list, and are not safe
either — §11.

### Queued requests

A request for an instance that is offline waits on the Hub. When the instance
sends `hello`, the queued requests are delivered **before** any new one, in the
order they were queued. A request that expired while it waited is delivered anyway
and answered `expired`; the instance, not the Hub, is the judge of the clock it
trusts (§6).

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
those bytes from the request it is about to send, then hashes and signs them with
its request-signing key. The instance parses the JSON, rebuilds the same bytes from
the parsed request, and verifies the signature against the key it pinned at pairing
(§4). The byte layout is specified once, in the module rustdoc of
[`canonical.rs`](../crates/demeteo-hub-protocol/src/canonical.rs) and on
`signing_bytes`, and is not repeated here.

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
| `request_id` | **single-use**. The instance records every id it accepts in its own SQLite (an id is not a secret) for at least as long as it can still be unexpired, and refuses a repeat, including across a restart |
| `expires_at` | checked against **the instance's clock**. An expired request is answered `expired` and has no effect |
| `instance_id` | must equal this instance's id, so one instance's request cannot be replayed at another |
| Order of checks | scope → signature → expiry → id unseen → kind-specific checks. The id is **spent when the signature verifies**, before the effect, and is **not refunded** if a later check refuses: a retry is a new request with a new id |

Refusal is the default. An unknown `kind`, a kind whose scope is off, or a body
that does not parse is answered `refused` and does nothing.

### 6.2 The six kinds

| Kind | Scope | Effect on the instance |
|---|---|---|
| `start_feature` | `spend` | start a Feature on a named Project and Workflow snapshot, with a description, optional attachments (§6.3) and optional per-step assignments. Refused locally if it would exceed a ceiling (§7) |
| `cancel_feature` | `spend` | cancel a running Feature. A `spend` request because it is run control, not an approval |
| `set_step_assignment` | `spend` | re-point one step's agent, model or effort mid-run. Exactly [decision 54](DECISIONS.md#54--mid-run-assignment-detail): tier 1 only, refused in `running` and `verifying`, refused for a kind that spawns no agent, refused for a harness the build has not registered |
| `gate_decision` | `gates` | decide a pending Gate. Carries a WebAuthn assertion; §8 |
| `passkey_endorsement` | none — authority is an assertion | add a passkey to the pinned set; §8 |
| `passkey_revocation` | none — authority is an assertion | remove a passkey from the pinned set; §8 |

`cancel_feature`'s scope is this document's assignment, not an interview answer
(§11). The two passkey kinds are not scope-gated because a scope toggles *what the
Hub may ask for*, and trust in a key is not something the Hub asks for: it is
granted only by a key the instance already trusts.

### 6.3 Attachments

An attachment is something the user attached to a Feature in the browser. It is the
one piece of user-authored content the Hub holds, and its lifecycle is built so the
Hub holds it as briefly and as opaquely as it can.

1. **Upload.** The browser sends the file to the Hub over TLS.
2. **Encrypt.** The Hub encrypts it before it is stored, as an envelope under
   OpenBao Transit: a per-attachment data key, itself wrapped by Transit, with the
   plaintext key discarded. Postgres holds ciphertext and a wrapped key, never
   plaintext.
3. **Reference.** The `start_feature` body names the attachment by id, size and the
   SHA-256 of the plaintext. No attachment bytes ride in an envelope.
4. **Fetch once.** The instance fetches it with its access token (a DPoP-bound
   request). The Hub unwraps the key through Transit and streams the plaintext.
   The instance checks the SHA-256 and refuses the request on a mismatch.
5. **Delete.** A completed fetch consumes the attachment: the Hub deletes it. An
   interrupted fetch may be retried until the request expires. A request that
   expires, is refused, or is cancelled before a fetch deletes its attachments
   too.

The Hub is not trusted with the attachment's integrity, which is why step 4 checks
a hash the envelope carries and the envelope is signed.

---

## 7. Scopes

| Scope | Allows | Requests |
|---|---|---|
| `read` | the instance sends events and snapshots | — |
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
coming; it does not ask permission.

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

The mockup shows no place to set a ceiling, and none for passkey management (§8).
Those surfaces are specified here with nothing to cite.

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

1. the Gate is **pending** on that `feature_id` and `step_execution_id`;
2. the assertion's `credential id` is in the **pinned set**;
3. `clientDataJSON.type` is `webauthn.get`, its `challenge` equals the challenge
   the instance **recomputes from the envelope**, and its `origin` is
   `https://` followed by the pinned RP ID;
4. `authenticatorData` carries the SHA-256 of the pinned RP ID, with the
   user-present and user-verified flags both set;
5. the signature verifies under the pinned public key over
   `authenticatorData ‖ SHA-256(clientDataJSON)`.

The instance recomputes the challenge instead of reading one from the Hub, so the
Hub cannot present an assertion for one decision as an assertion for another. It
does not rely on a signature counter: synced passkeys commonly report zero.

`decision` is `approve` or `cancel`. `redirect` carries free-text feedback that none
of the six fields binds, so a relaying Hub could rewrite what the human wrote; until
that is settled (§11) an instance refuses a `gate_decision` that is not `approve` or
`cancel`. It also never maps an unrecognised string to a verdict, because
`classify` in `domain/gate/decision.rs` cancels the run on one.

### The pinned set

The set **starts from the passkeys returned at pairing** (§4). It changes only by
two request kinds, and neither is authorised by the Hub:

- **`passkey_endorsement`** adds one. It carries the new credential's id and public
  key and an assertion, **by an already-trusted passkey, over the new credential's
  id and public key** (challenge prefix `demeteo-hub/v1/endorse`). The new key is
  trusted when the instance verifies that assertion, not before.
- **`passkey_revocation`** removes one. It carries an assertion **by another trusted
  passkey** over the credential to remove (prefix `demeteo-hub/v1/revoke`). A key
  cannot revoke itself, and **the last trusted key cannot be revoked**, so no
  request can leave the instance with nothing it trusts.

Both assertions bind `instance_id`, `request_id` and `expires_at` the way a gate's
does; the exact field lists are those of `endorsement_challenge` and
`revocation_challenge` in `canonical.rs`, and remain defaults (§11).

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

---

## 9. Storage, custody and layout

### Storage

**Postgres, plus OpenBao with its Transit engine for envelope encryption,** shipped
as Docker Compose with four services:

| Service | Role |
|---|---|
| `hub` | the axum server |
| `postgres` | tenants, passkey public keys, instance records, event store, snapshots, request queue, refresh-token hashes |
| `openbao` | custody; the Transit engine encrypts attachments and holds the Hub's secrets |
| `caddy` | TLS termination in front of the `hub` |

**Hub server secrets live in OpenBao or are encrypted under Transit — never
plaintext in Postgres.** That includes the request-signing key (§4) and the key
material for attachments (§6.3).

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
| `crates/demeteo-hub` | the axum server — **not built** |
| `crates/demeteo-hub-protocol` | **serde-only** wire types shared by the desktop client and the Hub — **not built** |
| `hub-web/` | a second Vite entry in this repo; it reuses `src/components/canvas` and the `src/App.css` tokens — **app shell only**: placeholder routes (Fleet, Instance, Runs, Run, Dispatch, AddInstance), no data wiring yet, and no view renders the canvas yet. `npm run build:hub-web` writes a static bundle to `hub-web/dist`, and `demeteo-hub` will serve the directory its configuration names |

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

**Runner dispatch is out of scope for this ticket set,** and a separate discovery
owns it. A `demeteo-runner` host is not paired with the Hub (decision 58), and no
request kind in §6 targets one.

What that discovery would pursue, recorded so it is not re-derived and **not
decided here**: the Hub drives runners over SSH the way a desktop does, using
per-run GitHub App installation tokens and GitLab OAuth short-lived access tokens
instead of stored PATs, and per-tenant SSH CAs held in the KMS that mint short-lived
user certificates. It builds a `RunSpec` from the last snapshot a paired desktop
reported (§5). It is gated on the SSH-certificate spike, ticket `hub-ssh-cert-spike`.

Two things about that plan are easy to get wrong:

- [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) says scoped-token minting is not
  implemented and standing PAT injection is what ships. That is still true. The
  per-run tokens above are the Hub's *future* answer, not the current runner's.
- **Rejected: the runner dialing out to the Hub.** It would need a Hub token at rest
  on a host with no keyring (the `keyring` feature note in
  `crates/demeteo-core/Cargo.toml`, and `crates/demeteo-runner/src/credentials.rs`),
  plus a second transport to the runner. **Rejected: the Hub storing PATs or
  long-lived SSH keys.**

The mockup's printed-code runner pairing
(`demeteo-runner hub pair <url>`) is superseded by this decision;
[`hub-design/README.md`](hub-design/README.md) lists it.

---

## 11. What is not decided

Open means unresolved. Nothing below is decided, and a build ticket that needs one
should raise it, not pick.

1. **Gate decisions other than `approve` and `cancel`.** `redirect` carries
   free-text feedback the challenge does not bind (§8). Whether the Hub offers it,
   and how the feedback would be bound, is open. Until then an instance refuses it.
2. **What a Gate's review pane may show on the Hub.** Transcripts, diffs and file
   contents never leave an instance (§3), so a Gate reviewed on the Hub is reviewed
   without them. Which gate summary text crosses, written by whom, is open.
3. **`cancel_feature`'s scope.** §6.2 assigns it `spend`; no interview answer
   decided it.
4. **What the rolling 24-hour total counts** — the budgets committed at start, or
   cost incurred — and its default. So does the default of the per-run ceiling,
   and whether `set_step_assignment` counts against either.
5. **A cursor gap.** If `run_events` ever gains a GC (none today), how the instance
   tells the Hub its cursor is older than what it holds (§5).
6. **Renewing the access token on an open socket**, since it lives 15 minutes and
   the socket does not, and what happens to the socket when the pairing is revoked.
7. **Hub-key rotation.** The request-signing key is pinned at pairing (§4); how a
   rotated key reaches an instance is not designed.
8. **A protocol version gate.** `hello` carries the version; what an instance and
   the Hub do on a mismatch is not decided. [Decision 55](DECISIONS.md#1-the-locked-decisions)
   is the runner's answer and is not assumed to carry over.
9. **Defaults carried, not measured:** ES256 for the device key, EdDSA for the
   envelope, 128 bits for a request id, the field lists of the two passkey
   challenges, and an upper bound on `expires_at` an instance will accept.
10. **Where the desktop-side client sits in the hexagon.** It must obey
    [AGENTS.md §3](../AGENTS.md) — no business logic in `commands/` — and which
    port it hangs from is a build ticket's call.
11. **Whether a step's `command` and prompt templates may cross at all.** They
    travel inside a Workflow snapshot's documents, and decision 59 needs them:
    a `RunSpec` cannot be built without the command and the prompt (§10). But
    both are user-authored text that may embed a local path or a credential
    (`curl -H "Authorization: Bearer …"`), which no field-level strip can
    detect. Sending them, redacting them, or sending a projection without them
    — the last re-decides decision 59 — is open. The snapshot builder's settled
    obligations, `cwd` and `remote_url` userinfo, are in §5; the Hub-client
    build ticket owns both those and this question.

---

## 12. Related

- [`DECISIONS.md`](DECISIONS.md) — decisions 56–63, the short form of this document
- [`hub-design/README.md`](hub-design/README.md) — the mockup, and where it is superseded
- [`MCP_INTEGRATION.md`](MCP_INTEGRATION.md) — the other external surface; §5 for the OAuth shape this one differs from, §8 for decision 50
- [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md) — §17, the web-companion bullet this reverses
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — the hexagon, and the crates this adds
- [`EXECUTION_PARITY.md`](EXECUTION_PARITY.md) — the contract a future runner dispatch must not fork
- [`REMOTE_EXECUTION.md`](REMOTE_EXECUTION.md) — runners, and the credential state §10 refers to
- [`../AGENTS.md`](../AGENTS.md) — §2 invariants, §6 Gate policy
