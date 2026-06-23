use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::ports::db::MachineRepository;
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;

pub struct RouterExecutionPort {
    machines: Arc<dyn MachineRepository>,
    ssh: Arc<dyn ExecutionPort>,
    local: Arc<dyn ExecutionPort>,
}

impl RouterExecutionPort {
    pub fn new(
        machines: Arc<dyn MachineRepository>,
        ssh: Arc<dyn ExecutionPort>,
        local: Arc<dyn ExecutionPort>,
    ) -> Self {
        Self {
            machines,
            ssh,
            local,
        }
    }

    fn resolve(&self, machine_id: &str) -> Result<Arc<dyn ExecutionPort>, String> {
        if machine_id.is_empty() || machine_id == "local" {
            return Ok(self.local.clone());
        }
        let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
            &*self.machines,
            machine_id,
        )?;
        match machine.auth_type.as_str() {
            "local" => Ok(self.local.clone()),
            _ => Ok(self.ssh.clone()),
        }
    }
}

#[async_trait]
impl ExecutionPort for RouterExecutionPort {
    async fn test_connection(&self, machine_id: &str) -> Result<(), String> {
        self.resolve(machine_id)?.test_connection(machine_id).await
    }

    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        self.resolve(machine_id)?.run_command(machine_id, cmd).await
    }

    async fn read_file(&self, machine_id: &str, path: &str) -> Result<String, String> {
        self.resolve(machine_id)?.read_file(machine_id, path).await
    }

    async fn write_file(&self, machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        self.resolve(machine_id)?
            .write_file(machine_id, path, content)
            .await
    }

    async fn get_metadata(&self, machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        self.resolve(machine_id)?
            .get_metadata(machine_id, path)
            .await
    }

    async fn list_dir(&self, machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        self.resolve(machine_id)?.list_dir(machine_id, path).await
    }

    async fn setup_worktree(
        &self,
        machine_id: &str,
        repo_path: &str,
        branch: &str,
        sandbox_path: &str,
    ) -> Result<(), String> {
        self.resolve(machine_id)?
            .setup_worktree(machine_id, repo_path, branch, sandbox_path)
            .await
    }

    async fn resolve_home(&self, machine_id: &str) -> Result<String, String> {
        self.resolve(machine_id)?.resolve_home(machine_id).await
    }

    fn spawn_interactive(
        &self,
        machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.resolve(machine_id)?
            .spawn_interactive(machine_id, binary, args, cwd, env)
    }
}
