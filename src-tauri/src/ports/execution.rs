use crate::sftp::SftpEntry;
use std::io;

/// A long-lived interactive process. The agent runtime owns both ends of
/// the stdio: a write half for the agent's stdin, and a read half for
/// stdout. The trait exposes blocking-style I/O so the CLI agent's
/// event-parsing loop can read stdout line-by-line.
pub trait InteractiveHandle: Send + Sync {
    fn write_line(&self, line: &str) -> std::io::Result<usize>;
    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn kill(&self) -> Result<(), String>;
    fn try_wait(&self) -> Result<Option<i32>, String>;
}

pub trait ExecutionPort: Send + Sync {
    /// Opens a TCP + SSH handshake + authentication against the machine and
    /// immediately closes it. Returns `Ok(())` on success, `Err(message)` on
    /// any connectivity or auth failure. Does NOT cache the session.
    fn test_connection(&self, machine_id: &str) -> Result<(), String>;
    fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String>;
    fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String>;
    fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String>;
    fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String>;
    fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String>;
    fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String>;

    /// Resolve the absolute home directory on the target host.
    fn resolve_home(&self, machine_id: &str) -> Result<String, String>;

    /// Spawn a long-lived interactive process on the target host and
    /// return an owned handle to its stdio. The local case uses
    /// `tokio::process::Command`; the SSH case uses a long-lived
    /// `ssh2::Channel` with PTY request for line-buffered stdout.
    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String>;
}
