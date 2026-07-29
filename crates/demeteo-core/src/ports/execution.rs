use async_trait::async_trait;
use serde::Serialize;
use std::collections::BTreeMap;

/// Prefix on the `Err` string of an [`ExecutionPort`] method when the
/// failure is a **transport/connection** problem (the machine could not be
/// reached, the SSH session dropped, auth failed) rather than a *command*
/// failure (the command ran and exited non-zero). Decision **D3**: callers
/// that need to distinguish "retry the connection" from "the command is
/// broken" test for this prefix; the payload after it is the underlying
/// message. A plain command failure carries stderr with no prefix.
pub const TRANSPORT_ERROR_PREFIX: &str = "transport: ";

/// Prefix on the `Err` string when the command was **abandoned at its
/// [`ShellOptions::timeout`]** rather than having run to a verdict. Kept
/// distinct from [`TRANSPORT_ERROR_PREFIX`] and from a bare non-zero exit
/// for the same reason D3 separates those two: a caller that treated a
/// timeout as a command verdict would redirect an agent to "fix" code that
/// never finished being tested. Both prefixes classify as an *environment*
/// failure; only an actual non-zero exit is a verdict.
pub const TIMEOUT_ERROR_PREFIX: &str = "timeout: ";

/// Explicit shell context for [`ExecutionPort::run_command_with`]. Every
/// field is data the caller supplies; **no adapter may fall back to ambient
/// process state** (the GUI's `PATH`/`HOME`/cwd). This is decision **D2** in
/// `docs/EXECUTION_PARITY.md`: two adapters given the *same*
/// `ShellOptions` must produce equivalent behaviour, so a command that
/// "works local, silently wrong on remote" can no longer exist.
///
/// [`ExecutionPort::run_command`] is the thin default — it is exactly
/// `run_command_with(.., ShellOptions::default())`, i.e. a non-login shell,
/// the adapter's default cwd, and no extra env.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShellOptions {
    /// When `true`, the command runs through a POSIX **login** shell
    /// (`bash -l -c`), so the target user's profile is sourced (`PATH`,
    /// `mise`/`asdf` shims, `~/.profile`, …). When `false` (the default),
    /// it runs through a plain non-login `sh -c` with no profile.
    ///
    /// This is the field that closes the remote-only PATH gap: the SSH
    /// adapter's historical bare `channel.exec` behaved like a non-login
    /// shell with an even thinner environment, so anything resolved via a
    /// profile-managed shim (`git`, `mise`-shimmed toolchains) was present
    /// locally and missing remotely. Callers that need the profile set
    /// `login_shell: true`; both adapters then honour it identically.
    pub login_shell: bool,
    /// When `true` (only meaningful alongside `login_shell`), the login shell
    /// is also **interactive** (`bash -l -i -c`), so the user's `~/.bashrc` is
    /// sourced in addition to the login profile.
    ///
    /// This is what actually closes the PATH gap for the common developer
    /// tool-managers — `mise`, `asdf`, `nvm`, `rbenv`, `pyenv` — whose
    /// `activate` hook lives in `~/.bashrc` behind the standard non-interactive
    /// guard (`case $- in *i*) ;; *) return;; esac`). A non-interactive login
    /// shell hits that guard and returns before the tool is put on `PATH`, so
    /// `command -v <tool>` reports "missing" even though an interactive login
    /// (what the user sees when they SSH in) finds it.
    ///
    /// Kept *opt-in and separate* from `login_shell` because an interactive
    /// shell sources the full `~/.bashrc`, which on some machines echoes a
    /// banner to stdout — fine for a probe or an agent spawn whose stdout is a
    /// stream, but corrupting for commands whose stdout is parsed
    /// (`resolve_home`, model probes). Only callers that need the tool-manager
    /// PATH (the availability probe, the agent spawn) set this.
    pub interactive: bool,
    /// Working directory the command runs in. `None` means "the adapter's
    /// default cwd" (local: the GUI process's cwd; SSH: the login
    /// directory). `Some(dir)` is honoured identically by every adapter.
    pub cwd: Option<String>,
    /// Extra environment variables exported *before* the command runs, on
    /// top of whatever the (login or non-login) shell itself establishes.
    /// This is the **only** caller-controlled environment; by contract
    /// nothing else crosses the transport boundary. A `BTreeMap` so the
    /// export order is deterministic (matters for conformance assertions
    /// and reproducible remote command strings).
    pub env: BTreeMap<String, String>,
    /// Wall-clock ceiling on the command. `None` (the default) means no
    /// ceiling.
    ///
    /// Enforced **by the adapter**, not by wrapping the returned future in
    /// `tokio::time::timeout`: both adapters do their work on the blocking
    /// pool, where dropping the future abandons the *wait* and leaves the
    /// process running. Owning the deadline is what lets an adapter actually
    /// stop the work — the local one kills the child's whole process group,
    /// so a `bash -c "npm test"` that hangs takes `npm` with it instead of
    /// orphaning it into a worktree that is about to be torn down.
    ///
    /// On expiry the call returns `Err` prefixed with
    /// [`TIMEOUT_ERROR_PREFIX`].
    ///
    /// Transport note: the SSH adapter can only abandon the wait — ssh2's
    /// API is synchronous and the remote process outlives the channel — so
    /// there the deadline bounds *demeteo's* wait, not the command. The
    /// error is identical either way.
    pub timeout: Option<std::time::Duration>,
}

impl ShellOptions {
    /// Convenience constructor for a login-shell context with no cwd/env
    /// override — the common "I need the user's PATH" case.
    pub fn login() -> Self {
        Self {
            login_shell: true,
            ..Self::default()
        }
    }

    /// Convenience constructor for an **interactive** login shell — the
    /// "I need the user's PATH *including* `mise`/`asdf`/`nvm` tools that are
    /// activated in `~/.bashrc`" case. See [`Self::interactive`].
    pub fn login_interactive() -> Self {
        Self {
            login_shell: true,
            interactive: true,
            ..Self::default()
        }
    }
}

#[derive(Serialize, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
}

/// A long-lived interactive process. The agent runtime owns both ends of
/// the stdio: a write half for the agent's stdin, and a read half for
/// stdout. The trait exposes blocking-style I/O so the CLI agent's
/// event-parsing loop can read stdout line-by-line.
///
/// Kept sync because the agent runtime spawns the handle from a sync
/// context (the SSH client opens a channel synchronously). The agent
/// runtime layer then bridges this into a tokio stream via
/// `tokio::task::spawn_blocking`.
pub trait InteractiveHandle: Send + Sync {
    fn write_line(&self, line: &str) -> std::io::Result<usize>;
    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn kill(&self) -> Result<(), String>;
    fn try_wait(&self) -> Result<Option<i32>, String>;
}

/// Async execution port. Every method returns a future; the
/// implementation is free to do its work on the calling runtime or
/// internally `spawn_blocking` if it touches synchronous I/O (e.g.
/// `ssh2`).
///
/// **Phase B (this migration):** Making the port async removed the
/// `tokio::task::spawn_blocking` wrappers that previously sat in every
/// command that called `run_command`. The synchronous `ssh2`/`std::fs`
/// calls now live inside the impl, where they belong.
///
/// # Behavioural contract (C0, `docs/EXECUTION_PARITY.md`)
///
/// This trait is the single behavioural contract every transport
/// (`LocalSubprocessAdapter`, `SshClientAdapter`, `RouterExecutionPort`)
/// must satisfy identically. It is enforced by the shared conformance suite
/// in `crates/demeteo-core/tests/conformance/execution_port.rs` — new
/// behaviour is added to that suite, not bug-hunted onto each path.
///
/// The three guarantees a caller may rely on regardless of transport:
///
/// 1. **Explicit context (D2).** Command execution never inherits ambient
///    state from the GUI process. Shell mode, cwd, and environment are
///    passed as [`ShellOptions`] and honoured identically. The bare
///    `run_command` is `run_command_with(.., ShellOptions::default())`.
/// 2. **Loud, uniform failure (D3).** A command that cannot run, or runs
///    non-zero, is always `Err` — never `Ok("")`. The `Err` string always
///    includes the captured stderr. A *transport/connection* failure (the
///    machine could not be reached) is distinguishable from a
///    *command* failure (it ran, exited non-zero) by the
///    [`TRANSPORT_ERROR_PREFIX`] on the former.
/// 3. **File ops mirror the shell contract.** `read_file` on a missing or
///    unreadable path is `Err`, never `Ok("")`; `list_dir` returns the
///    [`SftpEntry`] shape with `.`/`..` filtered and dirs-first ordering.
#[async_trait]
pub trait ExecutionPort: Send + Sync {
    /// Opens a TCP + SSH handshake + authentication against the machine and
    /// immediately closes it. Returns `Ok(())` on success, `Err(message)` on
    /// any connectivity or auth failure. Does NOT cache the session.
    async fn test_connection(&self, machine_id: &str) -> Result<(), String>;

    /// Run `cmd` through a **non-login** POSIX shell in the adapter's
    /// default cwd with no caller-supplied environment. Exactly equivalent
    /// to [`Self::run_command_with`] with [`ShellOptions::default`].
    ///
    /// Returns the command's stdout on a zero exit. On a non-zero exit the
    /// result is `Err` with the captured stderr attached (never `Ok("")`);
    /// on a transport failure the `Err` is prefixed with
    /// [`TRANSPORT_ERROR_PREFIX`]. See the trait-level contract.
    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        self.run_command_with(machine_id, cmd, ShellOptions::default())
            .await
    }

    /// Run `cmd` through a POSIX shell honouring `opts` — login vs non-login
    /// shell, working directory, and exported environment (see
    /// [`ShellOptions`]). Every adapter must honour every field identically;
    /// this is the method the conformance suite drives to prove local/SSH
    /// parity.
    ///
    /// Error semantics match [`Self::run_command`]: non-zero exit ⇒ `Err`
    /// with stderr; unreachable machine ⇒ `Err` prefixed with
    /// [`TRANSPORT_ERROR_PREFIX`].
    ///
    /// The default implementation delegates to [`Self::run_command`],
    /// **ignoring `opts`** — it exists only so simple test stubs need not
    /// implement both methods. Every real transport overrides it.
    async fn run_command_with(
        &self,
        machine_id: &str,
        cmd: &str,
        opts: ShellOptions,
    ) -> Result<String, String> {
        let _ = opts;
        self.run_command(machine_id, cmd).await
    }

    /// Read the UTF-8 contents of `path` on the target. A missing or
    /// unreadable path is `Err` (D3) — callers must not treat "no file" as
    /// an empty artifact.
    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String>;

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String>;

    /// Binary-safe variant of [`Self::write_file`] — `content` is
    /// arbitrary bytes, not necessarily valid UTF-8. Used to push the
    /// `demeteo-runner` executable itself over SFTP (M7.1); a `String`
    /// parameter can't carry an ELF binary.
    async fn write_file_bytes(
        &self,
        machine_id: &str,
        path: &str,
        content: &[u8],
    ) -> Result<(), String>;

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String>;

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String>;

    async fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String>;

    /// Resolve the absolute home directory on the target host.
    async fn resolve_home(&self, machine_id: &str) -> Result<String, String>;

    /// Resolve the SSH-authenticated username on the target host. The
    /// returned value matches what the remote shell's passwd entry
    /// would set `$USER` to — so the agent's `$USER` and `$HOME` are
    /// always internally consistent (HOME from `resolve_home`, USER
    /// from here, both derived from the same remote identity). For
    /// local machines (`""` or `"local"`) the value is the GUI
    /// process's own user.
    ///
    /// Returning a `Result` (rather than a default like the local
    /// `$USER`) makes a misconfigured machine loud: an empty
    /// `Machine.username` row surfaces as an error at agent-spawn
    /// time instead of silently forwarding the GUI's user to a
    /// remote box.
    async fn resolve_user(&self, machine_id: &str) -> Result<String, String>;

    /// Call the `demeteo-runner` control RPC on `machine_id`
    /// (docs/REMOTE_EXECUTION.md M6.1). Reaches
    /// `<home>/.local/share/demeteo-runner/control.sock` via OpenSSH
    /// Unix-socket forwarding (`channel_direct_streamlocal`, R4) — no new
    /// listening port, authz inherited from the SSH session. `method` /
    /// `params` / the returned value are the same newline-delimited-JSON
    /// shapes `demeteo-runner`'s RPC server speaks
    /// (`crates/demeteo-runner/src/rpc.rs`). Local machines have no
    /// runner (remote runs are Linux-remote-only, R2) and always error.
    async fn control_rpc(
        &self,
        machine_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String>;

    /// Spawn a long-lived interactive process on the target host and
    /// return an owned handle to its stdio. The local case uses
    /// `tokio::process::Command`; the SSH case uses a long-lived
    /// `ssh2::Channel` with PTY request for line-buffered stdout.
    ///
    /// Returns a sync [`InteractiveHandle`] because the agent runtime
    /// layer spawns the process synchronously and bridges into a tokio
    /// stream. The trait return type stays sync to avoid forcing every
    /// caller to box up the future for what is, semantically, a
    /// one-shot construction call.
    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String>;
}

/// Shared behavioural conformance suite (C2.1). Included here so it compiles
/// as part of the crate's `#[cfg(test)]` build and can reach the concrete
/// adapters via `crate::…`, matching the repo's existing `#[path]`-included
/// test convention.
#[cfg(test)]
#[path = "../../tests/conformance/execution_port.rs"]
mod conformance;

/// Topology-equivalence gate (C5): one workflow, every transport, one
/// equivalent `RunView`. Included the same `#[path]` way so it can reach the
/// composition root and concrete adapters via `crate::…`.
#[cfg(test)]
#[path = "../../tests/conformance/topology_equivalence.rs"]
mod topology_equivalence;

/// Harness-failure triage driver-integration fixtures (C6): a red harness in a
/// real running feature triages regression-vs-environment and routes each to
/// the right terminal path. Included the same `#[path]` way so it can drive the
/// engine via `crate::…` and reach the C5 stub runtime.
#[cfg(test)]
#[path = "../../tests/conformance/harness_triage.rs"]
mod harness_triage;

/// Multi-harness gating fixtures (HB5, `docs/HARNESS_BASELINE.md`): every
/// resolved gate runs even after one fails and each failure is attributed by
/// name, and a starter-shaped workflow that declares no harness is gated by the
/// project's selected validation gates. Runs a real shell; only the agent is
/// stubbed. Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/harness_gates.rs"]
mod harness_gates;

/// Baseline-measurement fixtures (HB2b / P4.2a, `docs/HARNESS_BASELINE.md`):
/// the in-graph node records the sha it actually measured, the lazy fallback
/// fires on validate's failure path and only there, a covering record is not
/// re-measured, a fallback that cannot measure leaves the verdict untouched,
/// and every worktree is torn down. Plus HB9's pair: a gate the measurement
/// found *unrunnable* ends the run at the head of the graph with remediation,
/// while a gate that is merely red walks the whole graph to be subtracted at
/// validate. Runs a real shell and real git; only the agent is stubbed.
/// Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/harness_baseline.rs"]
mod harness_baseline;

/// Subtraction fixtures (HB2c, `docs/HARNESS_BASELINE.md`): a gate red at the
/// base and identically red now does **not** fail the step, a gate red at the
/// base *because this machine cannot run it* **does** — terminally, with
/// remediation — a gate green at the base and red now does, a differently-red
/// gate does, and the exclusion is named in the evidence the validate turn is
/// handed. Runs a real shell and real git; only the agent is stubbed. Included
/// the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/harness_subtraction.rs"]
mod harness_subtraction;

/// Bootstrap harness-preflight gate (HB1, `docs/HARNESS_BASELINE.md`): a
/// project whose `test_command` names an unresolvable binary must be stopped at
/// launch, before any step row is seeded — and a project whose commands resolve
/// must be untouched. Runs a real shell for the false-positive leg.
#[cfg(test)]
#[path = "../../tests/conformance/preflight_gate.rs"]
mod preflight_gate;

/// Durable-checkpoint crash-resume gate (P1.9): a fresh driver life must
/// resume a checkpointed `sequence` step from the exact task, not the step
/// head. Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/durable_checkpoints.rs"]
mod durable_checkpoints;

/// Starter-baseline golden snapshots (P0.2, `docs/TASKS_DAG_WORKFLOWS.md`):
/// every bundled starter executed end-to-end under the stub agent and
/// compared to a committed behavioral snapshot — the regression gate for the
/// Phase-1 DAG engine rework. Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/starter_baseline.rs"]
mod starter_baseline;

/// Unified-event-log parity gate (P1.13): a local stub run's `run_events`
/// rows must replay into the same ordered story the live UI events told,
/// and the `run_event` live pushes must mirror the durable rows exactly.
/// Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/run_event_parity.rs"]
mod run_event_parity;

/// Stored-topology gate (P3.6): a version carrying a schema-v2 document
/// (V34) schedules the edges that document draws, not the chain its v1
/// projection flattens to. Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/stored_graph_topology.rs"]
mod stored_graph_topology;

/// `command` node gate (P3.5): a starter-shaped workflow with a command
/// node runs under the stub harness, fails like a harness on a non-zero
/// exit, and honors the PRD §5.4 idempotency rule on resume. Included the
/// same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/command_node.rs"]
mod command_node;

/// Resume fingerprint guard gate (P1.14): a workspace mutated between
/// crash and resume parks the interrupted node at the Decision-14
/// synthetic gate instead of blind re-execution; an untouched workspace
/// auto-resumes. Included the same `#[path]` way.
#[cfg(test)]
#[path = "../../tests/conformance/resume_fingerprint.rs"]
mod resume_fingerprint;
