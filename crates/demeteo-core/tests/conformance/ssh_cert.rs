//! Can libssh2 (`ssh2` 0.9.6) authenticate with an OpenSSH user certificate?
//! Driven by `ssh_cert/run-ssh-cert-spike.sh`, which owns the CA, the keys, the
//! sshd and the agents and exports `DEMETEO_SSH_CERT_*`. Findings:
//! `docs/HUB_SSH_CERT_SPIKE.md`.
//!
//! A "yes" is never read off a successful connect alone. The sshd auth log for
//! that attempt is parsed into [`AuthEvent`]s, and a cell is YES only when it
//! holds an `Accepted publickey` event for a certificate with a CA. Each
//! negative control asserts the *specific* rejection it exists to provoke, so a
//! rejection for an unrelated reason (a dropped connection, a banner failure)
//! cannot pass as proof that the oracle discriminates.
//!
//! The log is matched by structure, not by algorithm name: OpenSSH 10 logs
//! `ED25519-CERT`, older releases log `ssh-ed25519-cert-v01@openssh.com`, and
//! both are certificates. See [`parse_line`].

use std::fmt;
use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::thread::sleep;
use std::time::{Duration, Instant};

use ssh2::Session;

/// Upper bound on waiting for sshd to log the outcome of one attempt. Only an
/// attempt that sshd never logs (the harness lost the log, or the client died
/// before authenticating) pays it in full.
const LOG_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const LOG_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// One auth-relevant line of the sshd log, reduced to what the verdict reads.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AuthEvent {
    /// `Accepted publickey … <alg>-CERT … CA <type> SHA256:<fp>`.
    AcceptedCert {
        ca: String,
    },
    AcceptedPlain,
    /// `Failed publickey …`; `cert` is whether the offered key was a certificate.
    FailedPublickey {
        cert: bool,
    },
    /// `Certificate invalid: <reason>`, e.g. `expired`.
    CertRefused {
        reason: String,
    },
    /// `userauth_pubkey: signature algorithm <alg> not in PubkeyAcceptedAlgorithms`.
    AlgorithmRefused {
        algorithm: String,
    },
}

impl AuthEvent {
    fn is_accepted(&self) -> bool {
        matches!(self, Self::AcceptedCert { .. } | Self::AcceptedPlain)
    }
}

/// The key-type token after `ssh2:` and the `CA <type> <fingerprint>` tail are
/// the only parts shared by every OpenSSH version that logs these lines.
fn parse_line(line: &str) -> Option<AuthEvent> {
    if let Some((_, reason)) = line.split_once("Certificate invalid: ") {
        return Some(AuthEvent::CertRefused {
            reason: reason.trim().to_string(),
        });
    }
    if let Some((_, rest)) = line.split_once("signature algorithm ") {
        if let Some((algorithm, _)) = rest.split_once(" not in PubkeyAcceptedAlgorithms") {
            return Some(AuthEvent::AlgorithmRefused {
                algorithm: algorithm.to_string(),
            });
        }
    }

    let accepted = line.contains("Accepted publickey");
    if !accepted && !line.contains("Failed publickey") {
        return None;
    }
    let (_, tail) = line.split_once("ssh2: ")?;
    let tokens: Vec<&str> = tail.split_whitespace().collect();
    let cert = tokens
        .first()
        .is_some_and(|alg| alg.to_ascii_lowercase().contains("-cert"));
    if !accepted {
        return Some(AuthEvent::FailedPublickey { cert });
    }
    if !cert {
        return Some(AuthEvent::AcceptedPlain);
    }
    let ca = tokens
        .iter()
        .position(|t| *t == "CA")
        .and_then(|i| tokens.get(i + 1..i + 3))
        .map(|parts| parts.join(" "));
    // A certificate acceptance that names no CA is not evidence of CA trust.
    ca.map(|ca| AuthEvent::AcceptedCert { ca })
}

fn parse_log(slice: &str) -> Vec<(String, AuthEvent)> {
    slice
        .lines()
        .filter_map(|l| parse_line(l).map(|e| (l.to_string(), e)))
        .collect()
}

/// Whether sshd has logged the outcome of an attempt whose client-side result
/// was `ok`. An authenticated session must show an acceptance; a failed one
/// needs only some rejection, because a client may try several identities.
fn settled(events: &[AuthEvent], ok: bool) -> bool {
    if ok {
        events.iter().any(AuthEvent::is_accepted)
    } else {
        !events.is_empty()
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Rejection {
    /// A bare key offered to an sshd that trusts only a CA.
    PlainKey,
    /// A well-formed certificate signed by a CA sshd does not trust.
    UntrustedCa,
    /// A certificate from the trusted CA, outside its validity window.
    Expired,
}

impl Rejection {
    fn matched_by(self, events: &[AuthEvent]) -> bool {
        events.iter().any(|e| match (self, e) {
            (Self::PlainKey, AuthEvent::FailedPublickey { cert: false }) => true,
            (Self::UntrustedCa, AuthEvent::FailedPublickey { cert: true }) => true,
            (Self::Expired, AuthEvent::CertRefused { reason }) => reason.contains("expired"),
            _ => false,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Expect {
    /// Not a control: the matrix cell under test.
    Open,
    Rejected(Rejection),
    PlainAccepted,
}

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Yes,
    No,
    Invalid(&'static str),
    ControlOk,
    ControlFail(&'static str),
}

impl Verdict {
    fn is_harness_failure(&self) -> bool {
        matches!(self, Self::Invalid(_) | Self::ControlFail(_))
    }
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Yes => f.write_str("YES(cert)"),
            Self::No => f.write_str("NO"),
            Self::Invalid(why) => write!(f, "INVALID({why})"),
            Self::ControlOk => f.write_str("CONTROL-OK"),
            Self::ControlFail(why) => write!(f, "CONTROL-FAIL({why})"),
        }
    }
}

fn verdict(expect: Expect, ok: bool, events: &[AuthEvent]) -> Verdict {
    let accepted_cert = events
        .iter()
        .any(|e| matches!(e, AuthEvent::AcceptedCert { .. }));
    match (expect, ok) {
        (Expect::Open, true) if accepted_cert => Verdict::Yes,
        (Expect::Open, true) => Verdict::Invalid("connected without cert evidence"),
        (Expect::Open, false) => Verdict::No,
        (Expect::Rejected(_), true) => Verdict::ControlFail("accepted"),
        (Expect::Rejected(kind), false) if kind.matched_by(events) => Verdict::ControlOk,
        (Expect::Rejected(_), false) => Verdict::ControlFail("rejected for an unexpected reason"),
        (Expect::PlainAccepted, true) if accepted_cert => {
            Verdict::ControlFail("a certificate was accepted")
        }
        (Expect::PlainAccepted, true) if events.contains(&AuthEvent::AcceptedPlain) => {
            Verdict::ControlOk
        }
        (Expect::PlainAccepted, _) => Verdict::ControlFail("plain key not accepted"),
    }
}

struct Env {
    host: String,
    port: String,
    user: String,
    dir: PathBuf,
    log: PathBuf,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} not set; run via run-ssh-cert-spike.sh"))
}

impl Env {
    fn load() -> Self {
        Env {
            host: env("DEMETEO_SSH_CERT_HOST"),
            port: env("DEMETEO_SSH_CERT_PORT"),
            user: env("DEMETEO_SSH_CERT_USER"),
            dir: PathBuf::from(env("DEMETEO_SSH_CERT_DIR")),
            log: PathBuf::from(env("DEMETEO_SSH_CERT_LOG")),
        }
    }

    fn log_len(&self) -> usize {
        fs::read_to_string(&self.log).map(|s| s.len()).unwrap_or(0)
    }

    /// Polls the log from `from` until [`settled`] or [`LOG_SETTLE_TIMEOUT`].
    fn events_since(&self, from: usize, ok: bool) -> Vec<(String, AuthEvent)> {
        let deadline = Instant::now() + LOG_SETTLE_TIMEOUT;
        loop {
            let all = fs::read_to_string(&self.log).unwrap_or_default();
            let parsed = parse_log(all.get(from..).unwrap_or(""));
            let events: Vec<AuthEvent> = parsed.iter().map(|(_, e)| e.clone()).collect();
            if settled(&events, ok) || Instant::now() >= deadline {
                return parsed;
            }
            sleep(LOG_POLL_INTERVAL);
        }
    }

    fn session(&self) -> Session {
        let tcp = TcpStream::connect(format!("{}:{}", self.host, self.port)).expect("tcp connect");
        let mut sess = Session::new().expect("session");
        sess.set_tcp_stream(tcp);
        sess.set_timeout(10_000);
        sess.handshake().expect("handshake");
        sess
    }

    fn path(&self, variant: &str, name: &str) -> PathBuf {
        self.dir.join(variant).join(name)
    }
}

/// libssh2's agent client reads `SSH_AUTH_SOCK` from the process environment
/// and `ssh2` 0.9.6 offers no way to name a socket, so selecting an agent means
/// mutating process-global state. The lock makes a second test in this target
/// queue behind the first instead of racing it; the guard restores the prior
/// value so nothing leaks into a later attempt.
static AGENT_ENV: Mutex<()> = Mutex::new(());

struct AgentSock {
    previous: Option<std::ffi::OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl AgentSock {
    fn select(sock: &str) -> Self {
        let lock = AGENT_ENV.lock().unwrap_or_else(|p| p.into_inner());
        let previous = std::env::var_os("SSH_AUTH_SOCK");
        std::env::set_var("SSH_AUTH_SOCK", sock);
        AgentSock {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for AgentSock {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(v) => std::env::set_var("SSH_AUTH_SOCK", v),
            None => std::env::remove_var("SSH_AUTH_SOCK"),
        }
    }
}

struct Row {
    method: String,
    key: String,
    variant: String,
    detail: String,
    evidence: Vec<String>,
    verdict: Verdict,
}

fn attempt<F>(e: &Env, method: &str, key: &str, variant: &str, expect: Expect, f: F) -> Row
where
    F: FnOnce(&Session) -> Result<(), ssh2::Error>,
{
    let sess = e.session();
    let from = e.log_len();
    let res = f(&sess);
    let ok = res.is_ok() && sess.authenticated();
    let detail = match res {
        Ok(()) => "authenticated".to_string(),
        Err(err) => format!("{err}"),
    };
    let parsed = e.events_since(from, ok);
    let events: Vec<AuthEvent> = parsed.iter().map(|(_, ev)| ev.clone()).collect();
    Row {
        method: method.into(),
        key: key.into(),
        variant: variant.into(),
        detail,
        evidence: parsed.into_iter().map(|(line, _)| line).collect(),
        verdict: verdict(expect, ok, &events),
    }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|err| panic!("read {}: {err}", p.display()))
}

fn agent_sock(key: &str) -> Option<String> {
    std::env::var(format!("DEMETEO_SSH_CERT_AGENT_{}", key.to_uppercase())).ok()
}

#[test]
#[ignore = "needs run-ssh-cert-spike.sh (real sshd, CA, agents)"]
fn ssh_cert_matrix() {
    let e = Env::load();
    let u = e.user.clone();
    let mut rows: Vec<Row> = Vec::new();
    let keys = ["ed25519", "rsa", "ecdsa"];

    for key in keys {
        // (a) pubkey_file, cert as the public-key file
        rows.push(attempt(
            &e,
            "a:pubkey_file",
            key,
            "good",
            Expect::Open,
            |s| {
                s.userauth_pubkey_file(
                    &u,
                    Some(&e.path("good", &format!("{key}-cert.pub"))),
                    &e.path("good", key),
                    None,
                )
            },
        ));
        // (b) pubkey_memory, cert text as the public key
        rows.push(attempt(
            &e,
            "b:pubkey_memory",
            key,
            "good",
            Expect::Open,
            |s| {
                let cert = read(&e.path("good", &format!("{key}-cert.pub")));
                let privk = read(&e.path("good", key));
                s.userauth_pubkey_memory(&u, Some(&cert), &privk, None)
            },
        ));
        // (c1) userauth_agent exactly as ssh_util.rs calls it (first identity only)
        // (c2) every agent identity, in order
        if let Some(sock) = agent_sock(key) {
            let _agent = AgentSock::select(&sock);
            rows.push(attempt(
                &e,
                "c1:userauth_agent",
                key,
                "good",
                Expect::Open,
                |s| s.userauth_agent(&u),
            ));
            rows.push(attempt(
                &e,
                "c2:agent_each_identity",
                key,
                "good",
                Expect::Open,
                |s| {
                    let mut agent = s.agent()?;
                    agent.connect()?;
                    agent.list_identities()?;
                    let ids = agent.identities()?;
                    let mut last = None;
                    for id in &ids {
                        match agent.userauth(&u, id) {
                            Ok(()) => return Ok(()),
                            Err(err) => last = Some(err),
                        }
                    }
                    Err(last
                        .unwrap_or_else(|| ssh2::Error::from_errno(ssh2::ErrorCode::Session(-1))))
                },
            ));
            let mut agent_ids = Vec::new();
            let s = e.session();
            if let Ok(mut agent) = s.agent() {
                if agent.connect().is_ok() && agent.list_identities().is_ok() {
                    for id in agent.identities().unwrap_or_default() {
                        agent_ids.push(id.comment().to_string());
                    }
                }
            }
            println!("AGENT-IDENTITIES {key}: {agent_ids:?}");
        }
    }

    // Controls
    rows.push(attempt(
        &e,
        "ctl1:plain_no_cert(CA-only)",
        "ed25519",
        "good",
        Expect::Rejected(Rejection::PlainKey),
        |s| s.userauth_pubkey_file(&u, None, &e.path("good", "ed25519"), None),
    ));
    rows.push(attempt(
        &e,
        "ctl2a:untrusted_CA_cert",
        "ed25519",
        "badca",
        Expect::Rejected(Rejection::UntrustedCa),
        |s| {
            s.userauth_pubkey_file(
                &u,
                Some(&e.path("badca", "ed25519-cert.pub")),
                &e.path("badca", "ed25519"),
                None,
            )
        },
    ));
    rows.push(attempt(
        &e,
        "ctl2b:expired_cert",
        "ed25519",
        "expired",
        Expect::Rejected(Rejection::Expired),
        |s| {
            s.userauth_pubkey_file(
                &u,
                Some(&e.path("expired", "ed25519-cert.pub")),
                &e.path("expired", "ed25519"),
                None,
            )
        },
    ));
    rows.push(attempt(
        &e,
        "ctl3a:authorized_keys_file",
        "ctl",
        "ctl",
        Expect::PlainAccepted,
        |s| s.userauth_pubkey_file(&u, None, &e.path("ctl", "ctl"), None),
    ));
    rows.push(attempt(
        &e,
        "ctl3b:authorized_keys_memory",
        "ctl",
        "ctl",
        Expect::PlainAccepted,
        |s| s.userauth_pubkey_memory(&u, None, &read(&e.path("ctl", "ctl")), None),
    ));

    println!("\n==== MATRIX ====");
    for r in &rows {
        println!(
            "MATRIX | {:<26} | {:<8} | {:<8} | {:<44} | {}",
            r.method,
            r.key,
            r.variant,
            r.verdict.to_string(),
            r.detail
        );
    }
    println!("\n==== EVIDENCE (sshd log lines per attempt) ====");
    for r in &rows {
        println!("EVIDENCE {} {} {}:", r.method, r.key, r.variant);
        if r.evidence.is_empty() {
            println!("    (no sshd log lines)");
        }
        for l in &r.evidence {
            println!("    {l}");
        }
    }

    let bad: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict.is_harness_failure())
        .map(|r| format!("{} {} {}: {}", r.method, r.key, r.variant, r.verdict))
        .collect();
    assert!(
        bad.is_empty(),
        "harness invalid / controls failed: {bad:#?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEPTED_CERT_10: &str = "Accepted publickey for steven from 127.0.0.1 port 51234 ssh2: ED25519-CERT SHA256:aaaa ID cert-ed25519 (serial 0) CA ED25519 SHA256:cafp";
    const ACCEPTED_CERT_OLD: &str = "Accepted publickey for demeteo from 127.0.0.1 port 51234 ssh2: ssh-ed25519-cert-v01@openssh.com SHA256:aaaa ID cert-ed25519 (serial 0) CA ED25519 SHA256:cafp";
    const ACCEPTED_PLAIN: &str =
        "Accepted publickey for steven from 127.0.0.1 port 51234 ssh2: ED25519 SHA256:bbbb";
    const FAILED_PLAIN: &str =
        "Failed publickey for steven from 127.0.0.1 port 51234 ssh2: ED25519 SHA256:bbbb";
    const FAILED_CERT: &str = "Failed publickey for steven from 127.0.0.1 port 51234 ssh2: ED25519-CERT SHA256:aaaa ID cert-badca (serial 0) CA ED25519 SHA256:other";
    const EXPIRED: &str = "Refusing certificate ID \"cert-expired\" serial 0 (not valid): Certificate invalid: expired";
    const SHA1_RSA: &str = "userauth_pubkey: signature algorithm ssh-rsa-cert-v01@openssh.com not in PubkeyAcceptedAlgorithms [preauth]";

    fn ev(line: &str) -> AuthEvent {
        parse_line(line).unwrap_or_else(|| panic!("no event parsed from: {line}"))
    }

    #[test]
    fn parses_accepted_certificate_in_both_log_dialects() {
        let want = AuthEvent::AcceptedCert {
            ca: "ED25519 SHA256:cafp".into(),
        };
        assert_eq!(ev(ACCEPTED_CERT_10), want);
        assert_eq!(ev(ACCEPTED_CERT_OLD), want);
    }

    #[test]
    fn plain_acceptance_is_not_certificate_evidence() {
        assert_eq!(ev(ACCEPTED_PLAIN), AuthEvent::AcceptedPlain);
    }

    #[test]
    fn accepted_certificate_without_a_ca_is_not_parsed_as_evidence() {
        let line = "Accepted publickey for u from 127.0.0.1 port 1 ssh2: ED25519-CERT SHA256:aaaa ID x (serial 0)";
        assert_eq!(parse_line(line), None);
    }

    #[test]
    fn parses_rejections() {
        assert_eq!(ev(FAILED_PLAIN), AuthEvent::FailedPublickey { cert: false });
        assert_eq!(ev(FAILED_CERT), AuthEvent::FailedPublickey { cert: true });
        assert_eq!(
            ev(EXPIRED),
            AuthEvent::CertRefused {
                reason: "expired".into()
            }
        );
        assert_eq!(
            ev(SHA1_RSA),
            AuthEvent::AlgorithmRefused {
                algorithm: "ssh-rsa-cert-v01@openssh.com".into()
            }
        );
    }

    #[test]
    fn ignores_lines_that_are_not_auth_outcomes() {
        for line in [
            "Accepted certificate ID \"cert-ed25519\" (serial 0) signed by ED25519 CA SHA256:cafp via /tmp/ca.pub",
            "Connection from 127.0.0.1 port 51234 on 127.0.0.1 port 2299",
            "Received disconnect from 127.0.0.1 port 51234:11: disconnected by user",
            "",
        ] {
            assert_eq!(parse_line(line), None, "{line}");
        }
    }

    #[test]
    fn open_cell_is_yes_only_with_cert_evidence() {
        let cert = [ev(ACCEPTED_CERT_10)];
        assert_eq!(verdict(Expect::Open, true, &cert), Verdict::Yes);
        assert!(matches!(
            verdict(Expect::Open, true, &[]),
            Verdict::Invalid(_)
        ));
        assert!(matches!(
            verdict(Expect::Open, true, &[ev(ACCEPTED_PLAIN)]),
            Verdict::Invalid(_)
        ));
        assert_eq!(verdict(Expect::Open, false, &[]), Verdict::No);
    }

    #[test]
    fn rejection_control_requires_its_own_log_line() {
        let cases = [
            (Rejection::PlainKey, FAILED_PLAIN, FAILED_CERT),
            (Rejection::UntrustedCa, FAILED_CERT, FAILED_PLAIN),
            (Rejection::Expired, EXPIRED, FAILED_CERT),
        ];
        for (kind, right, wrong) in cases {
            let expect = Expect::Rejected(kind);
            assert_eq!(
                verdict(expect, false, &[ev(right)]),
                Verdict::ControlOk,
                "{kind:?}"
            );
            assert!(
                matches!(
                    verdict(expect, false, &[ev(wrong)]),
                    Verdict::ControlFail(_)
                ),
                "{kind:?} accepted the wrong rejection"
            );
            assert!(
                matches!(verdict(expect, false, &[]), Verdict::ControlFail(_)),
                "{kind:?} passed with no log evidence"
            );
            assert!(
                matches!(
                    verdict(expect, true, &[ev(ACCEPTED_CERT_10)]),
                    Verdict::ControlFail(_)
                ),
                "{kind:?} passed although the attempt authenticated"
            );
        }
    }

    #[test]
    fn plain_control_must_be_accepted_without_a_certificate() {
        let e = Expect::PlainAccepted;
        assert_eq!(verdict(e, true, &[ev(ACCEPTED_PLAIN)]), Verdict::ControlOk);
        assert!(matches!(
            verdict(e, true, &[ev(ACCEPTED_CERT_10)]),
            Verdict::ControlFail(_)
        ));
        assert!(matches!(verdict(e, true, &[]), Verdict::ControlFail(_)));
        assert!(matches!(
            verdict(e, false, &[ev(FAILED_PLAIN)]),
            Verdict::ControlFail(_)
        ));
    }

    #[test]
    fn settled_waits_for_the_outcome_that_matches_the_client_result() {
        assert!(!settled(&[], true));
        assert!(!settled(&[], false));
        // The agent loop logs a failed plain key before the accepted cert.
        assert!(!settled(&[ev(FAILED_PLAIN)], true));
        assert!(settled(&[ev(FAILED_PLAIN), ev(ACCEPTED_CERT_10)], true));
        assert!(settled(&[ev(FAILED_PLAIN)], false));
    }

    #[test]
    fn parse_log_keeps_the_raw_line_beside_its_event() {
        let slice = format!("Connection from x\n{FAILED_PLAIN}\n{ACCEPTED_CERT_10}\n");
        let parsed = parse_log(&slice);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, FAILED_PLAIN);
        assert!(parsed[1].1.is_accepted());
    }
}
