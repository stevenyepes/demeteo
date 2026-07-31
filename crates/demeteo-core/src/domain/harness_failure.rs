//! Did the command reach a verdict, and did it ever run?
//!
//! Two questions the driver must answer before it may call a red harness a red
//! build, and neither of them needs a port. The first reads the transport's own
//! error string; the second reads the command and its output for the shapes
//! that mean "nothing was tested" — a binary the shell could not find, or a
//! script this worktree does not define. Both were free functions buried in
//! `driver/verifier.rs`, decidable in a unit test but only reachable through
//! an adapter that carries twenty ports they never touch.
//!
//! The `crate::ports::execution` import below is for two `&str` constants, not
//! a trait; `domain/usage.rs` takes `ports::pricing::PricingTable` on the same
//! terms. The prefixes are the vocabulary the transports agree in, so the
//! constant has to be the one they raise.

/// How a failed `ExecutionPort` call on a prepare/harness command must be
/// answered. Pure over the error string, so the whole policy is decidable in a
/// unit test without a port double.
///
/// The distinction that matters is **"did the command reach a verdict?"**
/// Exactly one shape did: a non-zero exit. The other two were abandoned — the
/// machine went away, or the deadline expired — and a build that never finished
/// running is not a red build. Classifying either as a
/// [`Verdict`](crate::domain::verifier::VerifierError::Verdict) would redirect
/// an agent to "fix" code that was never tested, which is the exact failure
/// mode [`TRANSPORT_ERROR_PREFIX`](crate::ports::execution::TRANSPORT_ERROR_PREFIX)
/// and [`TIMEOUT_ERROR_PREFIX`](crate::ports::execution::TIMEOUT_ERROR_PREFIX)
/// exist to prevent (D3, `docs/EXECUTION_PARITY.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessExecFailure {
    /// The machine could not be reached or the channel broke.
    Transport,
    /// The command was abandoned at its `ShellOptions::timeout`.
    Timeout,
    /// The command ran and exited non-zero — the only shape that is a verdict.
    NonZeroExit,
}

pub fn classify_exec_failure(err: &str) -> HarnessExecFailure {
    if err.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX) {
        HarnessExecFailure::Transport
    } else if err.starts_with(crate::ports::execution::TIMEOUT_ERROR_PREFIX) {
        HarnessExecFailure::Timeout
    } else {
        HarnessExecFailure::NonZeroExit
    }
}

/// Detect "the shell could not find a binary the harness command invokes"
/// (exit 127) and return the missing command's name.
///
/// Recognizes the four diagnostics we can actually receive — dash/`sh`
/// (`sh: 1: cargo: not found`), bash (`bash: line 1: cargo: command not found`),
/// zsh (`zsh: command not found: cargo`), and Debian/Ubuntu's `command-not-found`
/// handler (`Command 'cargo' not found, but can be installed with:`) — because
/// the exit code itself is not reliably in the error string: the local adapter
/// formats `Command failed (exit code: Some(127)): …` but the SSH adapter
/// substitutes the remote stderr for the code whenever stderr is non-empty.
///
/// The `command-not-found` shape matters because it is the *default* on the
/// distro most remote boxes run: the handler is wired into bash's
/// `command_not_found_handle` hook and **replaces** the shell's own diagnostic,
/// so an Ubuntu machine never emits any of the three classic strings. Missing it
/// silently disabled this whole fast path there — the failure fell through to
/// the triage agent, which then parroted the handler's own `sudo apt install …`
/// suggestion as the remediation (usually the wrong toolchain entirely).
///
/// Guarded against false positives by requiring the missing name to appear as a
/// token of `cmd`. A test that merely *prints* "command not found" in its output
/// therefore stays a normal `Verdict`, and only a binary the harness genuinely
/// tries to run escalates. The cost of that guard is an indirect invocation
/// (`make test` shelling out to a missing `cargo`) not matching — that falls
/// through to the existing triage path, which reaches the same verdict one
/// attempt later.
pub fn detect_missing_command(cmd: &str, output: &str) -> Option<String> {
    let invoked = |name: &str| -> bool { command_invokes(cmd, name) };

    output.lines().map(str::trim).find_map(|line| {
        // Scan *within* the line rather than anchoring to its end: the SSH
        // adapter embeds the remote stderr mid-string (`Command failed (sh: 1:
        // cargo: not found): bash -l -i -c …`), so the diagnostic is not the
        // line's suffix.
        //
        // zsh (`command not found: npm`) is matched first because it names the
        // binary *after* the marker while carrying the bash marker's text as a
        // prefix — checking bash first would mis-extract the shell's own name.
        let raw = if let Some(name) = quoted_missing_command(line) {
            name
        } else if let Some((_, rest)) = line.split_once("command not found: ") {
            rest.split_whitespace().next()?
        } else if let Some(i) = line.find(": command not found") {
            line[..i].rsplit(':').next()?
        } else {
            let i = line.find(": not found")?;
            line[..i].rsplit(':').next()?
        };

        // Strip the punctuation an adapter's own wrapper can leave glued to the
        // name (`… command not found: npm): bash -l …`); no real binary ends in
        // one of these.
        let name = raw.trim().trim_end_matches([')', ':', ',', '.', '\'', '"']);

        invoked(name).then(|| name.to_string())
    })
}

/// Extract the binary name from Debian/Ubuntu's `command-not-found` handler,
/// which quotes the name instead of using the shell's `name: not found` shape:
///
/// ```text
/// Command 'cargo' not found, but can be installed with:
/// No command 'cargo' found, did you mean: …          (older releases)
/// ```
///
/// Both wordings are accepted, and the quote is matched loosely (`'` or `"`)
/// because the handler's own output has changed style across releases. The
/// caller still gates on the name being a token of the harness command, so a
/// loose match here cannot turn a red build into a false "environment" verdict.
fn quoted_missing_command(line: &str) -> Option<&str> {
    let (before, rest) = line.split_once(['\'', '"'])?;
    let before = before.trim_start();
    if !(before.starts_with("Command ") || before.starts_with("No command ")) {
        return None;
    }
    let (name, after) = rest.split_once(['\'', '"'])?;
    let after = after.trim_start();
    // "Command 'x' not found…" vs "No command 'x' found…" — one word each way.
    after
        .starts_with("not found")
        .then_some(name)
        .or_else(|| after.starts_with("found").then_some(name))
}

/// Does `cmd` name `name` as one of its own tokens?
///
/// The false-positive guard both "the command never ran" detectors share: a
/// name lifted out of *output* only counts when the harness command demonstrably
/// asks for it, so a suite that merely prints a diagnostic in its own output
/// stays a normal `Verdict` instead of terminating the step. Splitting on shell
/// separators as well as whitespace is what makes `cargo test;pytest` match
/// `pytest`, and comparing whole tokens is what keeps `cargo` from matching
/// `cargo-nextest`.
fn command_invokes(cmd: &str, name: &str) -> bool {
    !name.is_empty()
        && !name.contains(char::is_whitespace)
        && cmd
            .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')'))
            .any(|tok| tok == name)
}

/// A task runner that started fine but was asked for a script/target this
/// worktree does not define.
///
/// Distinct from a missing *binary* ([`detect_missing_command`]) in exactly one
/// way that matters: the remediation. A missing binary means "provision the
/// machine"; this means "the project's configured command and this worktree's
/// contents disagree", and telling a user to install something would send them
/// after a package that is already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingTask {
    /// The runner that reported it, as it appears in the harness command.
    pub runner: &'static str,
    /// The script / target name it could not find.
    pub name: String,
}

impl MissingTask {
    /// What the runner calls the thing it could not find — the word the user
    /// will search their own project for.
    pub fn noun(&self) -> &'static str {
        if self.runner == "make" {
            "target"
        } else {
            "script"
        }
    }

    /// A command that prints what this worktree *does* define, so the
    /// remediation ends with something to run rather than something to believe.
    fn list_command(&self) -> String {
        if self.runner == "make" {
            "grep -E '^[A-Za-z0-9_.-]+:' Makefile".to_string()
        } else {
            format!("{} run", self.runner)
        }
    }
}

/// The task runners whose missing-script wording does not name them, so the
/// harness command is what tells us which one spoke.
const TASK_RUNNERS: [&str; 4] = ["npm", "pnpm", "yarn", "bun"];

/// Detect "the runner started, but the script/target it was asked for does not
/// exist here" and return what was missing.
///
/// The second half of the exit-127 story. A missing binary and a missing script
/// are the same *category* — the command never ran, so no edit to the source can
/// turn it green — but only the first exits 127 and emits a shell diagnostic.
/// A missing script exits **1** and prints the runner's own wording, which is
/// indistinguishable from a red build to everything downstream: it became a
/// `Verdict`, fed the rework loop, and reproduced forever. That is precisely how
/// feature `f-1d0209a0e43d5b67` spent ~27 agent attempts on
/// `npm error Missing script: "checks:code"`.
///
/// Recognized wordings:
///
/// ```text
/// npm error Missing script: "checks:code"          npm ≥ 9
/// npm ERR! Missing script: "checks:code"           npm 7–8
/// npm ERR! missing script: checks:code             npm 6
/// ERR_PNPM_NO_SCRIPT  Missing script: checks:code  pnpm
/// error Command "checks:code" not found.           yarn 1
/// make: *** No rule to make target 'checks'.  Stop.
/// ```
///
/// # The false-positive guard
///
/// Escalating here is terminal, so a suite that merely *prints* one of these
/// strings must not trip it. Three conditions must hold together: the missing
/// name is a token of the harness command ([`command_invokes`], the same guard
/// the 127 path uses), the runner that would have reported it is *also* a token
/// of that command, and the text immediately preceding the marker reads as a
/// runner's own error preamble rather than prose quoting one
/// ([`is_runner_preamble`]). An assertion message like
/// `expected 'Missing script: "test"'` fails the third; a `cargo test` run fails
/// the second.
///
/// The cost, as with the 127 path, is indirect invocation: `make: *** No rule to
/// make target 'x', needed by 'y'` names a prerequisite the harness command
/// never mentions, so it falls through to the ordinary triage path and reaches
/// the same conclusion one attempt later. Documented, not accidental.
pub fn detect_missing_task(cmd: &str, output: &str) -> Option<MissingTask> {
    output.lines().map(str::trim).find_map(|line| {
        let lower = line.to_ascii_lowercase();

        // npm (every major) and pnpm: `… Missing script: "checks:code"`.
        if let Some(i) = lower.find("missing script:") {
            let (before, rest) = line.split_at(i);
            let name = clean_task_name(rest["missing script:".len()..].split_whitespace().next()?);
            let runner = runner_invoked_in(cmd, &TASK_RUNNERS)?;
            if is_runner_preamble(before) && name != runner && command_invokes(cmd, &name) {
                return Some(MissingTask { runner, name });
            }
            return None;
        }

        // yarn 1: `error Command "checks:code" not found.`
        //
        // The severity word is *required* here, unlike the two families below.
        // Debian's `command-not-found` handler opens a line with the very same
        // `Command 'x' not found` and means the opposite thing — a binary that
        // is not installed — which the 127 path owns and answers with the right
        // remediation. Demanding yarn's `error ` prefix is what keeps this
        // detector from claiming it.
        if let Some(i) = line.find("Command ") {
            let rest = &line[i + "Command ".len()..];
            if let Some(name) = quoted_token(rest, "not found") {
                let name = clean_task_name(name);
                let runner = runner_invoked_in(cmd, &TASK_RUNNERS)?;
                let before = line[..i].trim();
                if !before.is_empty()
                    && is_runner_preamble(before)
                    && name != runner
                    && command_invokes(cmd, &name)
                {
                    return Some(MissingTask { runner, name });
                }
                return None;
            }
        }

        // make: `make: *** No rule to make target 'checks'.  Stop.`
        const MAKE_MARKER: &str = "No rule to make target ";
        if let Some(i) = line.find(MAKE_MARKER) {
            let rest = &line[i + MAKE_MARKER.len()..];
            let name = clean_task_name(quoted_token(rest, "")?);
            if is_runner_preamble(&line[..i])
                && command_invokes(cmd, "make")
                && command_invokes(cmd, &name)
            {
                return Some(MissingTask {
                    runner: "make",
                    name,
                });
            }
        }

        None
    })
}

/// Which of `candidates` the harness command actually runs. `None` when it runs
/// none of them — the guard that keeps a `cargo test` whose output happens to
/// contain "Missing script" from being read as an npm failure.
fn runner_invoked_in(cmd: &str, candidates: &[&'static str]) -> Option<&'static str> {
    candidates.iter().copied().find(|r| command_invokes(cmd, r))
}

/// Is the text preceding a marker a task runner's *own* error preamble, rather
/// than prose that quotes one?
///
/// Anchoring on the start of the line is not an option: the execution adapters
/// wrap the failure (`Command failed (exit code: Some(1)): npm error Missing
/// script: …`), and the SSH adapter substitutes remote stderr for the exit code
/// entirely. So the check is on what sits immediately *before* the marker — a
/// runner's severity word, or nothing at all. A quote, a colon-space, or the
/// tail of an assertion message all fail it.
fn is_runner_preamble(before: &str) -> bool {
    let b = before.trim_end().to_ascii_lowercase();
    let b = b.trim_end_matches(':').trim_end();
    b.is_empty()
        || b.ends_with("error")
        || b.ends_with("err!")
        || b.ends_with("err_pnpm_no_script")
        // GNU make's own severity marker.
        || b.ends_with("***")
}

/// The quoted token at the head of `s`, when the text after the closing quote
/// starts with `expect` (pass `""` to accept anything).
///
/// Opening and closing quotes are matched loosely — `'x'`, `"x"`, and GNU make's
/// historical `` `x' `` all appear across tool and release — because the caller
/// still gates on the name being a token of the harness command, so a loose
/// match here cannot manufacture an escalation on its own.
fn quoted_token<'a>(s: &'a str, expect: &str) -> Option<&'a str> {
    let rest = s.trim_start().strip_prefix(['\'', '"', '`'])?;
    let (name, after) = rest.split_once(['\'', '"', '`'])?;
    after.trim_start().starts_with(expect).then_some(name)
}

/// Strip the quoting and trailing punctuation a runner leaves glued to the name
/// it could not find (`"checks:code"`, `'checks'.`). No real script or target
/// name ends in one of these.
fn clean_task_name(raw: &str) -> String {
    raw.trim()
        .trim_matches(['\'', '"', '`'])
        .trim_end_matches([')', ',', '.', ':', ';', '\'', '"', '`'])
        .to_string()
}

/// The user-facing message for a missing script/target.
///
/// A free function, not a method, so the wording is reachable from a test
/// without standing up an `ExecutionDriver` and the twenty ports it would not
/// read (AGENTS.md §3).
///
/// It deliberately does **not** reuse the 127 path's "make it discoverable on
/// PATH" remediation. The binary was found and did run; what is absent is a
/// script in *this worktree's* `package.json`/`Makefile`. The motivating
/// incident is the canonical shape of that: the project's test command had been
/// changed to `npm run checks:code` while the step's worktree was pinned to a
/// base commit predating the commit that added that script — so the setting and
/// the tree were each individually fine and only disagreed with each other.
/// Pointing the user at `apt`/`nvm` there would send them after a package that
/// was never missing.
pub fn build_missing_task_message(
    machine: &str,
    wt_path: &str,
    cmd: &str,
    missing: &MissingTask,
) -> String {
    let noun = missing.noun();
    crate::domain::harness_remediation::build_environment_message(
        machine,
        wt_path,
        cmd,
        &format!(
            "`{runner}` started fine but found no {noun} named `{name}` in this worktree, so the \
             command never ran and nothing was verified. This is not a verdict on the code — no \
             source edit can add a {noun} the command is not looking for.",
            runner = missing.runner,
            noun = noun,
            name = missing.name,
        ),
        &format!(
            "Nothing needs installing — the tool itself ran. The command comes from this \
             project's configured prepare/test command in its Demeteo project settings, and it \
             names a {noun} this worktree does not define. The usual cause is that the setting \
             was pointed at a {noun} added by a *later* commit while this step's worktree is \
             pinned to an older base commit, so `{name}` is genuinely absent here. List what \
             this worktree actually offers:\n\
             \x20 {list}\n\
             Then either point the project's prepare/test command at a {noun} that exists at \
             this worktree's base commit, or move the feature's base onto the commit that adds \
             `{name}`.",
            noun = noun,
            name = missing.name,
            list = missing.list_command(),
        ),
    )
}

#[cfg(test)]
#[path = "../../tests/domain/harness_failure/exec_failure.rs"]
mod exec_failure_tests;

#[cfg(test)]
#[path = "../../tests/domain/harness_failure/missing_command.rs"]
mod missing_command_tests;

#[cfg(test)]
#[path = "../../tests/domain/harness_failure/missing_task.rs"]
mod missing_task_tests;
