# Findings — libssh2 (`ssh2` 0.9.6) with OpenSSH user certificates

Gate decision: **approved (harden)**. Spike write-up: `docs/HUB_SSH_CERT_SPIKE.md`.

## What was tried

Question: can the `ssh2` crate `demeteo-core` already uses present a CA-signed OpenSSH
user certificate to an sshd that trusts the CA (`TrustedUserCAKeys`), so the hosted Hub
can log into a `demeteo-runner` without a long-lived per-machine key?

A harness generates a throwaway CA, keys and certificates (ed25519, RSA-3072,
ECDSA-P256), starts a real sshd and ssh-agents, and tries four auth methods per key type:
`userauth_pubkey_file` (a), `userauth_pubkey_memory` (b), `userauth_agent` as
`ssh_util.rs` calls it (c1), and a loop over every agent identity (c2). A cell is YES
only if the sshd log shows an `Accepted publickey` for a certificate with a CA. Controls:
plain key under CA-only trust, untrusted-CA cert, expired cert, and a plain key in
`authorized_keys`.

## What was learned

- **(a) and (b) work for all three key types**, using the `rsa-sha2-*` cert algorithms.
  The vendored libssh2 (`1.11.1_DEV`) already carries the cert algorithms. No new
  dependency, so no AGENTS.md §6 Gate.
- **The agent path is a trap.** `userauth_agent` offers only the first identity, which
  is the plain key (c1 is NO for every key type). Trying every identity (c2) works for
  ed25519 and ECDSA, but RSA signs with SHA-1 and a default OpenSSH 8.8+ sshd refuses it.
- **A log-token oracle depends on the OpenSSH version.** OpenSSH 10 logs `ED25519-CERT`,
  not the wire name.
- **`PerSourcePenalties` (OpenSSH 9.8+)** drops loopback connections after failed auths,
  and shows up as a handshake failure. A Hub that retries failed logins quickly would hit it.
- **Limits.** All results come from macOS aarch64 with OpenSSH 10.2 and a Unix-socket
  agent. The Docker backend and `Dockerfile.cert` have never been run, so there is no
  result against a Linux sshd (Debian bookworm ships OpenSSH 9.2).

## Recommendation

**HARDEN.** Build the Hub cert path on `userauth_pubkey_memory(user, Some(cert), key,
None)` with ed25519 keys, and do not use the agent. Close the Linux/Docker gap first.

## What the hardening step did

- Removed all five `// PROTOTYPE:` markers (`ssh_cert.rs`, `run-ssh-cert-spike.sh`,
  `Dockerfile.cert`).
- **Typed log matcher.** sshd lines are parsed into `AuthEvent`s (`parse_line`). A
  certificate acceptance without a `CA` tail is not evidence. The old substring checks
  are gone.
- **Negative controls assert their own rejection.** Plain key and untrusted CA need
  `Failed publickey` (bare / certificate); the expired cert needs `Certificate invalid:
  expired`. Before, any auth failure passed.
- **Polling replaces the 400 ms sleep.** The harness waits until the sshd log shows the
  outcome that matches the client result, with a 5 s cap.
- **`SSH_AUTH_SOCK` mutation is scoped.** libssh2's agent client reads it from the
  environment and `ssh2` 0.9.6 has no per-session socket, so it cannot be avoided.
  `AgentSock` serialises access behind a lock and restores the previous value on drop.
- **Tests that need no sshd.** Ten unit tests cover the parser (both OpenSSH log
  dialects, non-evidence lines) and the verdict rules (including the "connected without
  cert evidence" and "rejected for the wrong reason" cases). They run under a plain
  `cargo test -p demeteo-core`.
- **Verification.** Three deliberate breakages (a loosened `PlainKey` match, `Yes`
  without evidence, a parser that ignores `-cert`) each turned the new tests red; code
  restored. The local-backend harness was re-run: the matrix is unchanged (8 YES; c1 NO
  ×3; c2 RSA NO) and all five controls pass. Clippy with `-D warnings` is clean on the
  `ssh_cert` target.

## Not done

- **The `docker` backend is still unrun** (no daemon, no network here). The prototype
  comments were replaced with an explicit "unverified" note. Running it and recording
  the image digest and `sshd -V` remains the first follow-up.
- Not run: macOS/Windows `cross-os` CI or `scripts/check-windows.sh`. The test target is
  host-side and uses `std::env` and unix-style agent paths only through the shell script.
  The full `npm run checks` was not run, only fmt, clippy and the `ssh_cert` target.
- Not done, by scope: the cert auth kind in `ssh_util.rs`, a `conformance` job entry in
  `pr-checks.yml`, and the decision on agent-held certs. These stay as the follow-ups
  in the evaluation.
