use std::io::Read;

use crate::ports::execution::ExecutionPort;

pub async fn run_official_install(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    if machine_id == "local" || machine_id.is_empty() {
        run_local(install_command)
    } else {
        run_remote(exec, machine_id, install_command).await
    }
}

fn run_local(install_command: &str) -> Result<(), String> {
    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(install_command)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::shared::proc::sanitize_child_env(&mut command);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn install command: {}", e))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let mut out = String::new();
    let mut err = String::new();
    let _ = stdout.read_to_string(&mut out);
    let _ = stderr.read_to_string(&mut err);
    let status = child
        .wait()
        .map_err(|e| format!("Install wait failed: {}", e))?;
    if !status.success() {
        return Err(format!(
            "Install script failed (exit {:?}): {}{}",
            status.code(),
            err.trim(),
            if !out.is_empty() {
                format!("\nstdout: {}", out.trim())
            } else {
                String::new()
            }
        ));
    }
    Ok(())
}

async fn run_remote(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    install_command: &str,
) -> Result<(), String> {
    // Run the install command under an interactive login shell so the target
    // user's package managers (`npm` via nvm, `mise`, `asdf`, Homebrew) are on
    // `PATH` — the same shell mode the availability probe and agent spawn use.
    // A bare non-login shell here would fail to resolve an `npm i -g …` /
    // `mise use …` installer even though the tool is present for the user (D2).
    exec.run_command_with(
        machine_id,
        install_command,
        crate::ports::execution::ShellOptions::login_interactive(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/install.rs"]
mod tests;
