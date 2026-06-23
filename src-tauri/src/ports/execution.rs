use crate::sftp::SftpEntry;
use async_trait::async_trait;

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
#[async_trait]
pub trait ExecutionPort: Send + Sync {
    /// Opens a TCP + SSH handshake + authentication against the machine and
    /// immediately closes it. Returns `Ok(())` on success, `Err(message)` on
    /// any connectivity or auth failure. Does NOT cache the session.
    async fn test_connection(&self, machine_id: &str) -> Result<(), String>;

    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String>;

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String>;

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String>;

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
