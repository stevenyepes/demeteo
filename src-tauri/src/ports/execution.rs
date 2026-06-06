use crate::sftp::SftpEntry;

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
}
