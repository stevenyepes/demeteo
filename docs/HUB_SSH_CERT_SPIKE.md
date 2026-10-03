# Spike — libssh2 (`ssh2` 0.9.6) with OpenSSH user certificates

**Question.** Can the `ssh2` crate `demeteo-core` already uses present a CA-signed
OpenSSH user certificate to an sshd that trusts the CA through `TrustedUserCAKeys`,
so the hosted Hub can log into a `demeteo-runner` without a long-lived per-machine key?

**Answer.** Yes, for ed25519, RSA-3072 and ECDSA-P256, through
`userauth_pubkey_file` and `userauth_pubkey_memory`. Not through `userauth_agent` as
`ssh_util.rs` calls it today.

## Recommendation

Use **method (b), `userauth_pubkey_memory`**, with the cert text as the public key and
the Hub-issued ephemeral private key as the private key. It needs no key or cert on
disk, which suits ephemeral keys, and it covers all three key types. Method (a)
(`userauth_pubkey_file` with the cert path as the public-key file) works equally well
if a temp file is acceptable.

- No new dependency. Nothing was added to `Cargo.toml` or `package.json`
  beyond one `[[test]]` target, so the AGENTS.md §6 Gate is not triggered.
- Follow-up, not done here: add a cert auth kind to `connect()` in
  `crates/demeteo-core/src/ssh_util.rs` that calls `userauth_pubkey_memory(user,
  Some(cert), key, None)`.
- Do **not** build the cert path on `userauth_agent`. It only tries the first agent
  identity, which `ssh-add` makes the plain key, and for RSA the agent path signs with
  SHA-1 (see below).
- The Hub's CA-issued keys may be ed25519, ECDSA-P256 or RSA. Prefer ed25519: it has
  no SHA-1/SHA-2 algorithm trap.

## Matrix

Run on a real OpenSSH sshd, `local` backend, Unix-socket agent. "YES" means the
sshd logged `Accepted publickey … <TYPE>-CERT … ID <id> … CA <type> SHA256:<fp>` for
that attempt, not just that the client connected.

| Method | ed25519 | RSA-3072 | ECDSA-P256 |
|---|---|---|---|
| (a) `userauth_pubkey_file`, cert as pubkey file | YES | YES | YES |
| (b) `userauth_pubkey_memory`, cert as pubkey | YES | YES | YES |
| (c1) `userauth_agent` (as `ssh_util.rs` calls it) | NO | NO | NO |
| (c2) agent, try every identity in turn | YES | NO (default sshd) / YES (with SHA-1 cert alg allowed) | YES |

### Controls (all 3 behaved as expected)

| Control | Outcome | sshd log |
|---|---|---|
| Plain key, no cert, CA-only trust | rejected | `Failed publickey … ED25519 SHA256:…` |
| Cert from an untrusted CA | rejected | `Failed publickey … ED25519-CERT … ID cert-badca … CA ED25519 SHA256:<other>` |
| Expired cert | rejected | `Refusing certificate ID "cert-expired" … Certificate invalid: expired` |
| Same key in `authorized_keys` (file and memory) | accepted, no `-CERT` token | `Accepted publickey … ED25519 SHA256:…` |

### Evidence excerpt (method (b), ed25519)

```
Accepted certificate ID "cert-ed25519" (serial 0) signed by ED25519 CA SHA256:… via …/ca.pub
Accepted publickey for <user> from 127.0.0.1 port … ssh2: ED25519-CERT SHA256:<fp> ID cert-ed25519 (serial 0) CA ED25519 SHA256:<ca-fp>
```

## Findings

1. **(a) and (b) work for every key type.** libssh2 passes the cert blob through as
   the public key and signs with the private key. A second run restricted to
   `rsa-sha2-512-cert-v01@openssh.com,rsa-sha2-256-cert-v01@openssh.com` plus the
   ed25519 and ECDSA cert algorithms still accepted RSA through (a) and (b), so
   these paths use the SHA-2 cert algorithms, not SHA-1.
2. **`userauth_agent` tries only `identities[0]`** (`ssh2-0.9.6/src/session.rs`).
   `ssh-add key` with `key-cert.pub` beside it registers the plain key first and the
   cert second, so the cert is never offered. The sshd log shows only
   `Failed publickey … ED25519 SHA256:…` with no `-CERT` line.
3. **Agent path works if every identity is tried** (`agent.userauth(user, &identity)`
   in a loop): ed25519 and ECDSA succeed. This needs a code change to the agent arm.
4. **Agent path with RSA signs with SHA-1.** The sshd (OpenSSH 10.2) logs
   `userauth_pubkey: signature algorithm ssh-rsa-cert-v01@openssh.com not in
   PubkeyAcceptedAlgorithms`. With `PubkeyAcceptedAlgorithms +ssh-rsa-cert-v01@openssh.com`
   the same attempt succeeds, so the signature is valid and only the algorithm is
   refused. OpenSSH 8.8+ disables SHA-1 by default, so a modern runner rejects it.
5. **Oracle token differs by OpenSSH version.** OpenSSH 10 logs the short form
   `ED25519-CERT`, not `ssh-ed25519-cert-v01@openssh.com`. The harness parses each
   sshd log line into a typed `AuthEvent` (`ssh_cert.rs`, `parse_line`), treating a
   key-type token containing `-cert` plus a `CA <type> <fingerprint>` tail as a
   certificate. Anything greping the full name against a recent sshd sees nothing.
6. **Harness trap.** OpenSSH 9.8+ `PerSourcePenalties` drops connections from
   127.0.0.1 after a few failed auths. The first run died in `handshake` with
   "Failed getting banner" until the harness set `PerSourcePenalties no`. Runners
   see the same effect if the Hub retries failed logins quickly.
7. The vendored libssh2 is `1.11.1_DEV` and already lists the `*-cert-v01@openssh.com`
   algorithms in `hostkey.c`, which the prior expectation doubted.

## Versions

- `ssh2` 0.9.6, `libssh2-sys` 0.3.2, bundled libssh2 `1.11.1_DEV` (read from
  `libssh2-sys-0.3.2/libssh2/include/libssh2.h`).
- Client and server: `OpenSSH_10.2p1, LibreSSL 3.3.6` (macOS aarch64 host).
- Docker image digest: not recorded. The daemon was down and no network was available.

## What was not verified

- **The Docker backend.** `sshd/Dockerfile.cert` and the `docker` branch of the script
  were written to spec but never run. The matrix above comes from the `local`
  backend, a rootless sshd on `127.0.0.1:2299` run as the current user.
  `AuthorizedPrincipalsFile` maps the cert principal `demeteo` to that user, the same
  mechanism the image uses.
- **A Linux sshd.** All results come from macOS's OpenSSH 10.2. A Debian bookworm
  sshd (OpenSSH 9.2) is likely the same, but is unconfirmed.
- Windows and macOS agent transports (named pipe, launchd socket), as scoped out.

## Reproduce

```bash
crates/demeteo-core/tests/conformance/ssh_cert/run-ssh-cert-spike.sh
# Diagnostic variants (local backend only):
DEMETEO_CERT_SSHD_EXTRA='PubkeyAcceptedAlgorithms +ssh-rsa-cert-v01@openssh.com' …/run-ssh-cert-spike.sh
```

It exits 0 when every control is rejected for its own reason (plain key and untrusted
CA: `Failed publickey`; expired: `Certificate invalid: expired`) and no "YES" lacks cert
evidence. The log parser and verdict rules are plain functions with unit tests that need
no sshd: `cargo test -p demeteo-core --test ssh_cert`.
The matrix was identical across consecutive runs.

## If this had been refuted

The fallback would have been a per-machine key, or a library swap. Candidates would be
`russh` or shelling out to `ssh -o CertificateFile=…`. Adding either is an AGENTS.md §6
Gate item. This is moot, since (a) and (b) work.
