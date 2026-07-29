//! Re-establish-and-retry at the `ExecutionPort` boundary (S4,
//! `docs/RELIABILITY_PLAN.md`).
//!
//! A momentary network drop used to end a forty-minute pipeline: the pooled
//! session died, the reconnect inside [`SessionPool::get`] hit a five-second
//! TCP timeout against a network that was down for three, and the driver saw
//! nothing but a failed step. This module absorbs that blip — and refuses to
//! absorb anything else.
//!
//! # The one rule
//!
//! **A call is retried only when the remote was handed nothing.**
//!
//! That is the whole safety argument, and it is deliberately stronger than
//! "the error was a transport failure". `ExecutionPort::run_command_with`
//! executes arbitrary user shell — `git commit`, `npm publish`, a merge — and
//! the port carries no idempotency signal, so there is no way for this layer
//! to know whether re-running one is safe. What it *can* know is whether the
//! command was ever put on the wire: a session that could not be established
//! and a channel that could not be opened both mean the remote shell never saw
//! it. Re-running is then side-effect free **for every operation**, whatever
//! that operation does, which is why no per-method idempotency table is needed
//! and none exists.
//!
//! The converse is what [`FailureStage::RemoteMayHaveRun`] records. Once
//! `channel.exec` has been issued the command may be running *right now* —
//! ssh2 is synchronous and the remote process outlives the channel, so a drop
//! mid-drain leaves work in flight that a retry would duplicate. Those calls
//! are not retried even though they are unambiguously transport failures.
//! What that costs is stated in the plan entry: a drop in the middle of a
//! long command is still a failed step. What it buys is that this module can
//! never re-run a side effect.
//!
//! # What a caller sees
//!
//! Nothing, on success — the retry leaves no trace in the result, so a call
//! that recovered on attempt two is indistinguishable from one that worked
//! first time. On exhaustion the **last** failure's message is returned
//! verbatim, which keeps its [`TRANSPORT_ERROR_PREFIX`] and therefore its
//! routing: `classify_exec_failure` still answers `Transport`, the verifier
//! still routes to `Infrastructure`, and `preflight` still declines to read it
//! as a missing binary. Swallowing that distinction would silently reopen
//! every hole C0.2/D3 closed (`docs/EXECUTION_PARITY.md`).
//!
//! [`TRANSPORT_ERROR_PREFIX`]: crate::ports::execution::TRANSPORT_ERROR_PREFIX
//! [`SessionPool::get`]: super::session::SessionPool::get

use super::session::SessionPool;
use std::future::Future;
use std::time::{Duration, Instant};

/// Total attempts for one `ExecutionPort` call — the first plus every retry.
///
/// Small on purpose. Every attempt past the first costs a fresh connect, whose
/// own ceiling is the five-second TCP timeout in `ssh_util::connect` plus a
/// handshake, and the machines that will never come back pay that bill in full
/// before failing. Three attempts spread over [`BACKOFF`] covers the blip this
/// exists for (a few seconds of dropped network) while keeping the worst case
/// for a genuinely dead host in the low tens of seconds.
pub(super) const MAX_ATTEMPTS: u32 = 3;

/// How long to wait before attempt *n+1*. Indexed by the retry, not the
/// attempt: `BACKOFF[0]` precedes the second attempt.
///
/// The first wait is short because the common cause — a pooled session that
/// died while idle — is already resolved by the eviction that precedes the
/// retry, and there is nothing to wait *for*. The second is longer because a
/// failure that survived a re-establish is a real outage, and hammering it
/// helps nobody.
const BACKOFF: [Duration; (MAX_ATTEMPTS - 1) as usize] =
    [Duration::from_millis(500), Duration::from_millis(1500)];

/// The pause before `attempt` (1-based). Attempt 1 is immediate; anything past
/// the schedule reuses its last entry, so the array can be shortened or
/// lengthened without a panic waiting in it.
fn backoff_before(attempt: u32) -> Duration {
    if attempt <= 1 {
        return Duration::ZERO;
    }
    let idx = (attempt - 2) as usize;
    BACKOFF
        .get(idx)
        .copied()
        .unwrap_or_else(|| BACKOFF.last().copied().unwrap_or(Duration::ZERO))
}

/// How far a failed SSH operation got — the only question a retry decision is
/// allowed to ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FailureStage {
    /// The session could not be established, or a channel could not be opened
    /// on it. The remote was handed nothing, so re-running is side-effect free
    /// regardless of what the operation would have done. **The only retryable
    /// stage.**
    NeverReachedRemote,
    /// The transport broke after the remote accepted the work — a failed
    /// `exec`, a drain that timed out, a channel that would not close. The
    /// command may be running or already finished; a retry would duplicate it.
    /// Still a transport failure, still surfaced as one, never retried.
    RemoteMayHaveRun,
    /// Not a broken connection at all: the operation ran and this `Err` is its
    /// answer (a non-zero exit, a missing file), or the failure is local to
    /// this process (a poisoned lock, a panicked blocking task). Retrying
    /// would re-ask a question that has already been answered.
    Answered,
}

/// A failed SSH operation, carrying the message the port will surface plus the
/// one fact a retry needs.
///
/// The message is passed through untouched — a `transport:`-prefixed failure
/// exhausts its attempts and then still reads as a transport failure to
/// `classify_exec_failure`.
pub(super) struct SshFailure {
    pub(super) message: String,
    pub(super) stage: FailureStage,
}

impl SshFailure {
    /// The remote was handed nothing. See [`FailureStage::NeverReachedRemote`].
    pub(super) fn never_reached(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stage: FailureStage::NeverReachedRemote,
        }
    }

    /// The remote may have begun the work. See
    /// [`FailureStage::RemoteMayHaveRun`].
    pub(super) fn may_have_run(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stage: FailureStage::RemoteMayHaveRun,
        }
    }

    /// The operation answered, or this process failed locally. See
    /// [`FailureStage::Answered`].
    pub(super) fn answered(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            stage: FailureStage::Answered,
        }
    }
}

/// Obstacles a reconnect cannot clear, matched case-insensitively against the
/// failure message.
///
/// Everything here is a *configuration or credential* problem raised by
/// `ssh_util::connect` or `resolve_machine` on the way to a connection, which
/// [`FailureStage::NeverReachedRemote`] would otherwise make retryable. Two
/// reasons to exclude them: the second and third attempts cannot possibly
/// succeed, and repeating a rejected authentication is actively harmful —
/// that is how an account gets locked or an IP lands in `fail2ban`.
///
/// Deliberately short. A handshake failure is *not* here: a restarting sshd
/// produces one and a retry a second later is exactly right. The list only
/// names failures whose cause is a value the user typed.
const PERMANENT_MARKERS: &[&str] = &[
    "authentication failed",
    "machine not found",
    "unknown auth type",
    "ssh connection is not applicable",
    "private key file not found",
    "key path points to a public key",
    "ssh password is required",
    "private key path is required",
    "has no username configured",
];

/// Does `message` name something a reconnect cannot fix? Pure, so the list
/// above is decidable in a unit test.
pub(super) fn names_a_permanent_obstacle(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    PERMANENT_MARKERS.iter().any(|m| lowered.contains(m))
}

/// The retry decision, in one place and free of I/O.
pub(super) fn is_retryable(failure: &SshFailure) -> bool {
    failure.stage == FailureStage::NeverReachedRemote
        && !names_a_permanent_obstacle(&failure.message)
}

/// Run `attempt` until it succeeds, until a failure is not retryable, or until
/// [`MAX_ATTEMPTS`] is spent — evicting the pooled session between tries so the
/// next one genuinely re-establishes rather than reusing the corpse.
///
/// `limit` is the caller's [`ShellOptions::timeout`] and bounds the **whole
/// call including its retries**, not each attempt. That is the deliberate
/// choice S7 warns about: a verifier that asked for a 1800s ceiling gets 1800s,
/// not `attempts × 1800`, so S10's deadline survives this change intact. The
/// expiry message is byte-identical to the local adapter's, because both
/// transports must classify the same way.
///
/// Cancellation needs no machinery here. The backoff is a plain `.await`, so a
/// caller that races this against `cancel_watch` (as `run_harness_command`
/// does) drops the future and the wait ends immediately — Stop stays prompt
/// *during* a retry, which is the S10 bug this must not resurrect in a new
/// place.
///
/// [`ShellOptions::timeout`]: crate::ports::execution::ShellOptions::timeout
pub(super) async fn with_ssh_retry<T, F, Fut>(
    op: &'static str,
    machine_id: &str,
    pool: &SessionPool,
    limit: Option<Duration>,
    mut attempt: F,
) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, SshFailure>>,
{
    let started = Instant::now();
    let deadline = limit.map(|l| started + l);

    for n in 1..=MAX_ATTEMPTS {
        let remaining = match deadline {
            None => None,
            Some(d) => match d.checked_duration_since(Instant::now()) {
                // `checked_duration_since` is `None` once the deadline is in
                // the past, which is the shape of "there is no time left".
                None => return Err(ceiling_message(limit)),
                Some(left) => Some(left),
            },
        };

        let outcome = match remaining {
            Some(left) => match tokio::time::timeout(left, attempt()).await {
                Ok(finished) => finished,
                Err(_) => return Err(ceiling_message(limit)),
            },
            None => attempt().await,
        };

        let failure = match outcome {
            Ok(value) => {
                if n > 1 {
                    tracing::info!(
                        op,
                        machine = machine_id,
                        attempt = n,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "ssh call recovered after re-establishing the session",
                    );
                }
                return Ok(value);
            }
            Err(failure) => failure,
        };

        if !is_retryable(&failure) {
            if n > 1 {
                // Reached only when an earlier attempt *was* retryable, so the
                // log has to say the retrying stopped for a new reason rather
                // than leaving the previous "retrying" line as the last word.
                tracing::warn!(
                    op,
                    machine = machine_id,
                    attempt = n,
                    stage = ?failure.stage,
                    error = %failure.message,
                    "ssh retry abandoned: this failure is not one a reconnect can clear",
                );
            }
            return Err(failure.message);
        }

        if n == MAX_ATTEMPTS {
            tracing::warn!(
                op,
                machine = machine_id,
                attempts = n,
                elapsed_ms = started.elapsed().as_millis() as u64,
                error = %failure.message,
                "ssh call failed after exhausting every reconnect attempt",
            );
            return Err(failure.message);
        }

        let delay = backoff_before(n + 1);
        // Never sleep into the caller's ceiling. Surfacing the transport
        // failure we already hold beats manufacturing a timeout: it is the
        // more specific answer and it is the one D3 routes correctly.
        if let Some(d) = deadline {
            if Instant::now() + delay >= d {
                tracing::warn!(
                    op,
                    machine = machine_id,
                    attempts = n,
                    error = %failure.message,
                    "ssh retry abandoned: the caller's ceiling leaves no room for another attempt",
                );
                return Err(failure.message);
            }
        }

        // The pooled session is why a retry can work at all. `get`'s liveness
        // probe is an SFTP `readdir`, which a half-open connection can still
        // answer while `channel_session` fails — so without this the retry
        // would reuse the very session that just failed and fail identically.
        pool.evict(machine_id);
        tracing::warn!(
            op,
            machine = machine_id,
            attempt = n,
            max_attempts = MAX_ATTEMPTS,
            backoff_ms = delay.as_millis() as u64,
            error = %failure.message,
            "ssh transport failed before the remote was reached; re-establishing and retrying",
        );
        tokio::time::sleep(delay).await;
    }

    // Unreachable while `MAX_ATTEMPTS >= 1` — the final iteration always
    // returns. Written as an `Err` rather than an `unreachable!` because a
    // panic here would take down a step for a constant somebody edited, and
    // the prefix keeps even this impossible answer on the transport path.
    Err(format!(
        "{}ssh call made no attempt (MAX_ATTEMPTS is {})",
        crate::ports::execution::TRANSPORT_ERROR_PREFIX,
        MAX_ATTEMPTS,
    ))
}

/// The caller's ceiling expired. Byte-identical to the local adapter's message
/// (`adapters/local/execution.rs`) so `classify_exec_failure` and every log
/// reader see one transport-independent shape.
fn ceiling_message(limit: Option<Duration>) -> String {
    format!(
        "{}command exceeded its {}s ceiling",
        crate::ports::execution::TIMEOUT_ERROR_PREFIX,
        limit.map(|l| l.as_secs()).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::step_executor::driver::verifier::{
        classify_exec_failure, HarnessExecFailure,
    };
    use crate::domain::ids::{AgentProfileId, MachineId};
    use crate::domain::models::{AgentProfile, Machine};
    use crate::ports::db::MachineRepository;
    use crate::ports::execution::TRANSPORT_ERROR_PREFIX;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// A repository that answers nothing. Every method is an `Err`, so a test
    /// that accidentally reaches the machine lookup fails loudly instead of
    /// being handed a plausible default — the shape AGENTS.md §7 warns about,
    /// where a double that returns success for everything makes a suite that
    /// cannot fail. `with_ssh_retry` only ever asks the pool to *evict*, which
    /// touches no repository, so a reachable lookup would mean the retry loop
    /// had grown an I/O path it is not supposed to have.
    struct NoMachines;
    impl MachineRepository for NoMachines {
        fn get_machines(&self) -> Result<Vec<Machine>, String> {
            Err("the retry loop must not query the machine repository".to_string())
        }
        fn get_machine(&self, _: &MachineId) -> Result<Option<Machine>, String> {
            Err("the retry loop must not query the machine repository".to_string())
        }
        fn add(&self, _: Machine) -> Result<(), String> {
            Err("the retry loop must not write machines".to_string())
        }
        fn update(&self, _: Machine) -> Result<(), String> {
            Err("the retry loop must not write machines".to_string())
        }
        fn delete(&self, _: &MachineId) -> Result<(), String> {
            Err("the retry loop must not write machines".to_string())
        }
        fn get_agent_profiles(&self, _: &MachineId) -> Result<Vec<AgentProfile>, String> {
            Err("the retry loop must not read agent profiles".to_string())
        }
        fn add_agent_profile(&self, _: AgentProfile) -> Result<(), String> {
            Err("the retry loop must not write agent profiles".to_string())
        }
        fn delete_agent_profile(&self, _: &AgentProfileId) -> Result<(), String> {
            Err("the retry loop must not write agent profiles".to_string())
        }
    }

    fn pool() -> SessionPool {
        SessionPool::new(Arc::new(NoMachines))
    }

    /// The message a dropped connection actually arrives as: tagged by
    /// `transport_err`, which is what makes the D3 routing work.
    fn dropped() -> String {
        format!("{TRANSPORT_ERROR_PREFIX}Failed to open SSH channel: Unable to send channel-open request")
    }

    /// The blip this whole module exists for: the first attempt cannot reach
    /// the remote, the second one can. The caller must see an ordinary
    /// `Ok` — the retry leaves no trace in the result, because a caller that
    /// could tell would start branching on it.
    #[tokio::test]
    async fn a_drop_before_the_remote_was_reached_is_retried_and_succeeds_transparently() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();

        let out = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                match seen.fetch_add(1, Ordering::SeqCst) + 1 {
                    1 => Err(SshFailure::never_reached(dropped())),
                    2 => Ok("survived-the-blip".to_string()),
                    n => panic!("the script has no answer for attempt {n}"),
                }
            }
        })
        .await;

        assert_eq!(out, Ok("survived-the-blip".to_string()));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "expected exactly one retry"
        );
    }

    /// The regression guard for the entire verifier chain. A connection that
    /// never comes back must exhaust its attempts and *still* be a transport
    /// failure — asserted through `classify_exec_failure`, not by eyeballing
    /// the string, because that classifier is the thing downstream depends on:
    /// `Transport` routes to `VerifierError::Infrastructure`, which is
    /// non-retryable and deliberately not a verdict. A retry wrapper that
    /// rewrote this into any other class would send an agent to "fix" code
    /// that was never tested.
    #[tokio::test]
    async fn a_persistent_drop_still_classifies_as_transport_after_every_attempt() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();

        let err = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::never_reached(dropped()))
            }
        })
        .await
        .expect_err("a host that never comes back must fail");

        assert_eq!(
            classify_exec_failure(&err),
            HarnessExecFailure::Transport,
            "exhausted retries must still route to Infrastructure, got: {err}",
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            MAX_ATTEMPTS,
            "the attempt count must be bounded by MAX_ATTEMPTS",
        );
    }

    /// The command ran and exited non-zero. That is an answer, not a broken
    /// connection, and re-running it would burn the caller's ceiling to
    /// re-learn the same thing — worse, for a command with side effects it
    /// would repeat them.
    #[tokio::test]
    async fn a_command_that_ran_and_failed_is_never_retried() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();

        let err = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::answered(
                    "Command failed (exit code: 1): sh -c 'cargo test'".to_string(),
                ))
            }
        })
        .await
        .expect_err("a non-zero exit is still an error");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a verdict must not be re-run"
        );
        assert_eq!(
            classify_exec_failure(&err),
            HarnessExecFailure::NonZeroExit,
            "the verdict must reach the caller unchanged: {err}",
        );
    }

    /// The line the safety argument rests on. This *is* a transport failure —
    /// same prefix, same routing — but the remote already accepted the
    /// command, so it may be running right now. Re-running it would duplicate
    /// whatever it does, and the port carries no idempotency signal that could
    /// say otherwise.
    #[tokio::test]
    async fn a_drop_after_the_remote_accepted_the_command_is_never_retried() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();

        let err = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::may_have_run(format!(
                    "{TRANSPORT_ERROR_PREFIX}Connection appears dead: no data and no keepalive \
                     response for 120s while waiting for command stdout"
                )))
            }
        })
        .await
        .expect_err("a dead connection mid-command is still an error");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "work that may already be running must not be re-issued",
        );
        assert_eq!(classify_exec_failure(&err), HarnessExecFailure::Transport);
    }

    /// A rejected credential reaches the retry loop as `NeverReachedRemote` —
    /// nothing ran — but repeating it cannot succeed and can lock the account
    /// or trip `fail2ban`. It is the one stage-retryable shape that must stop
    /// at one attempt.
    #[tokio::test]
    async fn a_rejected_credential_is_not_retried() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();

        let _ = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::never_reached(format!(
                    "{TRANSPORT_ERROR_PREFIX}Password authentication failed: \
                     [-18] Username/PublicKey combination invalid"
                )))
            }
        })
        .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Stop must stay prompt *during* a retry wait, which is the S10 bug in a
    /// new place if it regresses. Real time here on purpose: the point is that
    /// dropping the future ends the backoff immediately, and a paused clock
    /// would make any implementation look instant.
    #[tokio::test]
    async fn cancelling_during_the_backoff_wait_returns_promptly() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let started = Instant::now();

        let retrying = with_ssh_retry("run_command", "m1", &pool, None, || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::never_reached(dropped()))
            }
        });

        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            _ = retrying => panic!("the retry loop outran the cancel"),
        }

        assert!(
            started.elapsed() < backoff_before(2),
            "the cancel must not have waited out the backoff, took {:?}",
            started.elapsed(),
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "no second attempt may start after the caller has dropped the call",
        );
    }

    /// S7's warning, pinned: the caller's ceiling is spent across the **whole**
    /// call, retries and backoff included, never restarted per attempt. A
    /// ceiling that renewed itself would silently multiply the verifier
    /// deadline S10 exists to enforce by `MAX_ATTEMPTS`.
    ///
    /// The attempts have to *consume* time for this to be observable: with
    /// instant failures both designs stop at the same place, which is exactly
    /// how the first version of this test passed against a per-attempt ceiling.
    /// Each attempt here burns 800ms of a 1500ms ceiling, so a renewed budget
    /// runs to ~2100ms and finishes with a transport error, while a shared one
    /// is cut off at the ceiling and reports a timeout.
    #[tokio::test]
    async fn the_callers_ceiling_bounds_the_whole_call_not_each_attempt() {
        let pool = pool();
        let limit = Duration::from_millis(1500);
        let per_attempt = Duration::from_millis(800);
        let started = Instant::now();

        let err = with_ssh_retry("run_command", "m1", &pool, Some(limit), || async move {
            tokio::time::sleep(per_attempt).await;
            Err::<String, _>(SshFailure::never_reached(dropped()))
        })
        .await
        .expect_err("a persistent drop under a ceiling must still fail");

        let elapsed = started.elapsed();
        assert!(
            elapsed < limit + Duration::from_millis(300),
            "the whole retried call must fit the caller's ceiling, took {elapsed:?}",
        );
        assert_eq!(
            classify_exec_failure(&err),
            HarnessExecFailure::Timeout,
            "a call cut off at its ceiling reports a timeout, not a verdict: {err}",
        );
    }

    /// The other ceiling branch: there is time left, but not enough for another
    /// backoff. Giving up there must surface the transport failure already in
    /// hand rather than manufacturing a timeout — it is the more specific
    /// answer, and it is the one D3 routes to `Infrastructure`.
    #[tokio::test]
    async fn a_ceiling_with_no_room_for_another_attempt_surfaces_the_transport_failure() {
        let pool = pool();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let limit = Duration::from_millis(1200);
        let started = Instant::now();

        let err = with_ssh_retry("run_command", "m1", &pool, Some(limit), || {
            let seen = seen.clone();
            async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<String, _>(SshFailure::never_reached(dropped()))
            }
        })
        .await
        .expect_err("a persistent drop under a ceiling must still fail");

        let elapsed = started.elapsed();
        assert!(
            elapsed <= limit,
            "giving up early must not spend the ceiling, took {elapsed:?}",
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "the second backoff does not fit, so there is no third attempt",
        );
        assert_eq!(
            classify_exec_failure(&err),
            HarnessExecFailure::Transport,
            "giving up early must still name the transport failure we hold: {err}",
        );
    }

    /// The bound has to be small in absolute terms, not merely finite. Every
    /// attempt past the first costs a fresh connect — up to the 5s TCP timeout
    /// in `ssh_util::connect` plus a handshake — and this loop sits under
    /// `preflight`, the Machines view's HOME probe and every step of a run, so
    /// a generous-looking bump to the constants is paid by a user watching a
    /// dead host fail to report. Pinning the worst case is the only thing that
    /// makes such a bump visible in review.
    #[test]
    fn the_retry_budget_stays_small_enough_to_bound_the_worst_case() {
        assert!(
            (2..=4).contains(&MAX_ATTEMPTS),
            "one retry is the point and four attempts is already 15s of connects; got {MAX_ATTEMPTS}",
        );
        let waiting: Duration = BACKOFF.iter().take((MAX_ATTEMPTS - 1) as usize).sum();
        assert!(
            waiting <= Duration::from_secs(5),
            "a fully exhausted retry must not add more than a few seconds of pure waiting, got {waiting:?}",
        );
    }

    /// When the ceiling expires *inside* an attempt the answer is a timeout,
    /// not a verdict — the same distinction `TIMEOUT_ERROR_PREFIX` exists for,
    /// and the same message the local adapter emits so the two transports
    /// classify identically.
    #[tokio::test]
    async fn a_ceiling_that_expires_mid_attempt_reports_a_timeout() {
        let pool = pool();
        let err = with_ssh_retry(
            "run_command",
            "m1",
            &pool,
            Some(Duration::from_secs(1)),
            || async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Err::<String, _>(SshFailure::never_reached(dropped()))
            },
        )
        .await
        .expect_err("a command past its ceiling must fail");

        assert_eq!(err, "timeout: command exceeded its 1s ceiling");
        assert_eq!(classify_exec_failure(&err), HarnessExecFailure::Timeout);
    }

    /// The schedule is finite and ordered, and reading past its end must
    /// saturate rather than panic — the array is a tuning knob and shortening
    /// it should not plant an index panic in the transport.
    #[test]
    fn the_backoff_schedule_is_ordered_and_saturates() {
        assert_eq!(
            backoff_before(1),
            Duration::ZERO,
            "the first try is immediate"
        );
        assert!(backoff_before(2) > Duration::ZERO);
        assert!(
            backoff_before(3) > backoff_before(2),
            "a failure that survived a reconnect must wait longer",
        );
        assert_eq!(
            backoff_before(99),
            backoff_before(MAX_ATTEMPTS),
            "reading past the schedule must saturate, not panic",
        );
    }

    /// The permanence list has to catch the credential and configuration
    /// failures a reconnect cannot clear, and must **not** catch a handshake
    /// failure — a restarting sshd produces one and retrying a second later is
    /// exactly the behaviour this module is for.
    #[test]
    fn permanent_obstacles_are_named_and_a_handshake_failure_is_not_one() {
        for permanent in [
            "transport: Password authentication failed: [-18]",
            "transport: SSH agent authentication failed: [-16]",
            "transport: Machine not found: box-7",
            "transport: Private key file not found: /home/a/.ssh/id_ed25519",
        ] {
            assert!(
                names_a_permanent_obstacle(permanent),
                "must not be retried: {permanent}",
            );
        }

        for transient in [
            "transport: SSH handshake failed: [-43] Failed getting banner",
            "transport: Cannot reach box:22 (timeout after 5s) — connection refused",
            "transport: Failed to open SSH channel: Unable to send channel-open request",
        ] {
            assert!(
                !names_a_permanent_obstacle(transient),
                "must stay retryable: {transient}",
            );
        }
    }

    /// The retry decision in one assertion: stage decides, and permanence can
    /// only ever veto.
    #[test]
    fn only_a_never_reached_failure_without_a_permanent_cause_is_retryable() {
        assert!(is_retryable(&SshFailure::never_reached(dropped())));
        assert!(!is_retryable(&SshFailure::may_have_run(dropped())));
        assert!(!is_retryable(&SshFailure::answered("exit code: 1")));
        assert!(!is_retryable(&SshFailure::never_reached(
            "transport: Password authentication failed",
        )));
    }
}
