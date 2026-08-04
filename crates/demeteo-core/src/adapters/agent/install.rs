use crate::ports::execution::{ExecutionPort, ShellOptions};

/// Run an agent's official installer on the machine that agent will run on.
///
/// One call for every transport, deliberately. The installer is a
/// user-authored shell one-liner (`npm i -g …`, `curl … | bash`), so it takes
/// the user-authored plane's `run_command_with` and whatever shell that
/// transport resolved. The local branch this replaces spawned `sh` by name,
/// which on Windows names nothing at all — `sh.exe` is inside the Git
/// installation and not on `PATH` — so local installation there could not run,
/// and the plane split (`docs/WINDOWS_PARITY.md`) is what removes the question.
///
/// `login_interactive` is the same shell mode `availability()` probes under and
/// the same one the agent is later spawned from. Anything weaker resolves a
/// different `PATH`: an installer that lands its binary through `nvm`/`mise`/
/// `asdf` writes where only that shell looks, so the install reports success
/// and the probe immediately afterwards reports the agent missing.
///
/// The error carries the installer's own output verbatim. A human pressed a
/// button and is waiting on this one, and `start_with_install` puts the string
/// in front of them — a summary here is a support request later.
pub async fn run_official_install(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    exec.run_command_with(
        machine_id,
        install_command,
        ShellOptions::login_interactive(),
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("Install script failed: {}\ncommand: {}", e, install_command))
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/install.rs"]
mod tests;
