//! Whether a launch may proceed, and how the answer reads.

/// What a configured command has to survive, stated once and rendered
/// everywhere it applies.
///
/// The engine already tells a user this when a baseline cannot be measured
/// (`adapters::step_executor::baseline`); the settings panel needs the identical
/// sentence *before* they have paid for a run. Both sites therefore read this
/// constant instead of each carrying its own wording — a second copy would
/// drift, and the two would then disagree about the two things nobody guesses:
/// that the worktree a harness runs in is a fresh `git worktree add` with no
/// `node_modules` and no `target/` (which is why `prepare_command` exists at
/// all), and that a watch-mode runner never exits, so it burns the entire
/// wall-clock ceiling and then fails.
pub const FRESH_CHECKOUT_REMEDIATION: &str =
    "Run the command below in a *fresh* checkout — that is what this step gets, with no \
     `node_modules` and no `target/`. If it needs an install step, set the project's \
     prepare command; if it hangs, it is most likely a watch-mode runner, which never \
     exits.";

/// Why a machine that unquestionably has Git still cannot run one configured
/// command, and what to install so that it can.
///
/// Whoever meets this has a working git — every clone and fetch Demeteo made on
/// this machine went through it — so a bare "no shell found" reads as nonsense
/// and sends them auditing a `PATH` that is fine. The cause is narrower than
/// that and has a name, so the text says the name.
///
/// One constant for the reason [`FRESH_CHECKOUT_REMEDIATION`] is one: the
/// launch refusal and the settings panel render the identical sentence, and a
/// second copy would drift until the two disagreed about which install is
/// broken.
pub const MISSING_POSIX_SHELL_REMEDIATION: &str =
    "Git is installed on this machine, but Git Bash is not — so there is no POSIX shell to run \
     the project's commands with. Demeteo runs one POSIX script body on every platform, and on \
     Windows that body needs the bash the full Git for Windows package installs. A MinGit \
     install — the trimmed git that some tools bundle — ships git.exe without it, which is why \
     nothing else has complained. The run is stopped here because nothing downstream can supply \
     a missing interpreter: the validate step would fail on the same thing after the whole \
     implementation had been paid for.\n\
     Install Git for Windows from https://git-scm.com/download/win with its default options, \
     then check again. If bash is already installed somewhere the search does not look, set \
     DEMETEO_BASH_PATH to its full path (…\\bin\\bash.exe) and Demeteo will use that instead of \
     searching.";

/// What the preflight established about a project's configured commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightVerdict {
    /// The project configures **no command at all** — no `prepare_command`, no
    /// `test_command`, and no named harness. Nothing to probe — **not** a
    /// failure. A project may legitimately have no harness; the run proceeds
    /// and the validate step is told plainly that nothing ran (S12).
    ///
    /// It is deliberately *all three*: a project that configures only named
    /// harnesses has a harness, and reporting "none configured" at it would be
    /// false.
    NotConfigured,
    /// Every binary the configured commands name resolves on the login shell's
    /// `PATH`. Carries what was actually checked, so the phase detail can say
    /// so rather than claiming more than it verified.
    Resolved { probed: Vec<String> },
    /// At least one named binary does not resolve. The run is blocked: nothing
    /// downstream can turn "not installed" into a passing harness, and every
    /// step until `s-validate` would be paid for before anyone found out.
    MissingBinaries { missing: Vec<String> },
    /// There is no POSIX shell on the machine to run any configured command
    /// with. Blocks for the same reason [`MissingBinaries`] does, and one step
    /// earlier: the probe could not even ask, so no command has a verdict and
    /// none can be given one downstream.
    ///
    /// [`MissingBinaries`]: PreflightVerdict::MissingBinaries
    MissingPosixShell,
}

impl PreflightVerdict {
    /// Whether the launch may proceed.
    pub fn permits_launch(&self) -> bool {
        !matches!(
            self,
            PreflightVerdict::MissingBinaries { .. } | PreflightVerdict::MissingPosixShell
        )
    }

    /// The error the feature terminates with, or `None` to proceed.
    ///
    /// Not "`detail` when there is one": [`NotConfigured`] carries a detail
    /// the stepper shows and the launch must ignore, which is the whole
    /// asymmetry this type exists for. Only a blocked verdict answers, and it
    /// answers with the same sentence the stepper rendered, so the terminal
    /// error and the phase the user watched fail can never disagree.
    ///
    /// [`NotConfigured`]: PreflightVerdict::NotConfigured
    pub fn launch_refusal(&self) -> Option<String> {
        if self.permits_launch() {
            return None;
        }
        Some(
            self.detail()
                .unwrap_or_else(|| "harness preflight failed".to_string()),
        )
    }

    /// The `BootstrapProgress` status this verdict renders as.
    pub fn phase_status(&self) -> &'static str {
        match self {
            // Nothing to check is not the same as checked-and-fine, and the
            // stepper distinguishes them: `skipped` reads as "you have no
            // harness", which is information the user may want to act on.
            PreflightVerdict::NotConfigured => "skipped",
            PreflightVerdict::Resolved { .. } => "completed",
            PreflightVerdict::MissingBinaries { .. } | PreflightVerdict::MissingPosixShell => {
                "failed"
            }
        }
    }

    /// The human-facing `detail` line for the stepper, and — on the failing
    /// path — the error the feature terminates with. Names the binary and how
    /// to check it, because "not found" without a reproduce line sends people
    /// looking in the wrong shell.
    pub fn detail(&self) -> Option<String> {
        match self {
            PreflightVerdict::NotConfigured => Some(
                "This project configures no commands at all — no test command, no prepare \
                 command, and no named harnesses — so nothing will be run to verify the \
                 feature. Set at least a test command in project settings if you want the \
                 validate step to have evidence to judge."
                    .to_string(),
            ),
            PreflightVerdict::Resolved { probed } if probed.is_empty() => None,
            PreflightVerdict::Resolved { probed } => {
                Some(format!("Resolved on PATH: {}", probed.join(", ")))
            }
            PreflightVerdict::MissingBinaries { missing } => Some(format!(
                "The project's configured commands name {plural} the login shell cannot find: \
                 {list}. The run is stopped here because nothing downstream can make {them} \
                 appear — the validate step would fail on the same thing after the whole \
                 implementation had been paid for.\n\
                 Check with:\n\
                 \x20 bash -l -i -c 'command -v {first}'\n\
                 If that prints nothing, either export the tool's directory from ~/.profile or \
                 ~/.bashrc, or — if a version manager owns it (mise, asdf, nvm, pyenv, rbenv) — \
                 declare it in that manager's *global* config so every shell activates it. If \
                 the command itself is wrong, fix it in project settings.",
                plural = if missing.len() == 1 {
                    "a binary"
                } else {
                    "binaries"
                },
                list = missing.join(", "),
                them = if missing.len() == 1 { "it" } else { "them" },
                first = missing.first().map(String::as_str).unwrap_or(""),
            )),
            PreflightVerdict::MissingPosixShell => {
                Some(MISSING_POSIX_SHELL_REMEDIATION.to_string())
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/harness_preflight/verdict.rs"]
mod tests;
