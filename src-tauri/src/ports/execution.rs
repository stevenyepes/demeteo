use crate::sftp::SftpEntry;
use std::io;

/// A long-lived interactive process. The agent runtime owns both ends of
/// the stdio: a write half for the agent's stdin, and a read half for
/// stdout. The trait exposes blocking-style I/O because the JSON-RPC
/// layer is line-buffered and synchronous; `kill` and `try_wait` let the
/// runtime clean up without depending on the process exiting on its own.
///
/// Implementations are responsible for honoring `kill` (close pipes,
/// send SIGTERM/EOF) and for surfacing EOF to the reader via a
/// short-read (0 bytes) on stdout.
pub trait InteractiveHandle: Send {
    /// Write a single line (the caller is responsible for including the
    /// trailing newline if the protocol expects it). Returns the number
    /// of bytes written.
    fn write_line(&mut self, line: &str) -> io::Result<usize>;

    /// Read bytes from stdout. Returns Ok(0) on EOF; Ok(n) with n>0 on
    /// a partial read. The JSON-RPC layer reads byte-by-byte until
    /// newline, so partial reads are fine.
    fn read_byte(&mut self) -> io::Result<u8>;

    /// Read available bytes into `buf` without blocking. Returns Ok(0)
    /// on EOF and Ok(n) on partial read.
    fn try_read(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Block until at least one byte is available or EOF is hit.
    /// Returns Ok(0) on EOF, Ok(n) on read.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;

    /// Send EOF / SIGTERM to the agent and close pipes. Idempotent.
    fn kill(&mut self) -> Result<(), String>;

    /// If the agent has exited, return its exit code; else return None.
    /// The watchdog polls this between read timeouts to detect death.
    fn try_wait(&mut self) -> Result<Option<i32>, String>;
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
    fn setup_worktree(&self, machine_id: &str, repo_path: &str, branch: &str, sandbox_path: &str) -> Result<(), String>;

    /// Resolve the absolute home directory on the target host.
    ///
    /// Implementations MUST return an absolute path (no `~`) and SHOULD
    /// cache the result per `machine_id` because the value is stable for
    /// the lifetime of the SSH connection. The local adapter returns the
    /// process `HOME`; the SSH adapter runs `echo $HOME` over a channel
    /// so the value matches the remote user's actual home regardless of
    /// how the demeteo process was launched.
    fn resolve_home(&self, machine_id: &str) -> Result<String, String>;

    /// Spawn a long-lived interactive process on the target host and
    /// return an owned handle to its stdio. The local case uses
    /// `tokio::process::Child`; the SSH case uses a long-lived
    /// `ssh2::Channel`.
    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String>;
}
