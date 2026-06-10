use std::collections::HashMap;
use std::process::Command;

use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;

pub struct LocalSubprocessAdapter;

impl LocalSubprocessAdapter {
    pub fn new() -> Self {
        Self
    }
}

fn local_run_command(cmd: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| format!("Failed to execute command: {}", e))?;

    let mut result = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
        return Err(format!("Command failed (exit code: {:?}): {}", output.status.code(), result));
    }

    Ok(result)
}

impl ExecutionPort for LocalSubprocessAdapter {
    fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn run_command(&self, _machine_id: &str, cmd: &str) -> Result<String, String> {
        local_run_command(cmd)
    }

    fn read_file(&self, _machine_id: &str, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read file '{}': {}", path, e))
    }

    fn write_file(&self, _machine_id: &str, path: &str, content: &str) -> Result<(), String> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directories: {}", e))?;
        }
        std::fs::write(path, content)
            .map_err(|e| format!("Failed to write file '{}': {}", path, e))
    }

    fn get_metadata(&self, _machine_id: &str, path: &str) -> Result<SftpEntry, String> {
        let path_buf = std::path::Path::new(path);
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("Failed to stat '{}': {}", path, e))?;

        let name = path_buf.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let modified = meta.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(SftpEntry {
            name,
            path: path.to_string(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified,
        })
    }

    fn list_dir(&self, _machine_id: &str, path: &str) -> Result<Vec<SftpEntry>, String> {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("Failed to read directory '{}': {}", path, e))?;

        let mut list = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path_buf = entry.path();
            let name = path_buf.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            if name == "." || name == ".." {
                continue;
            }

            let meta = entry.metadata().map_err(|e| format!("Failed to read metadata: {}", e))?;
            let modified = meta.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            list.push(SftpEntry {
                name,
                path: path_buf.to_str().unwrap_or("").to_string(),
                is_dir: meta.is_dir(),
                size: meta.len(),
                modified,
            });
        }

        list.sort_by(|a, b| {
            if a.is_dir != b.is_dir {
                b.is_dir.cmp(&a.is_dir)
            } else {
                a.name.cmp(&b.name)
            }
        });

        Ok(list)
    }

    fn setup_worktree(&self, _machine_id: &str, repo_path: &str, branch: &str, sandbox_path: &str) -> Result<(), String> {
        local_run_command(&format!("mkdir -p {}/.demeteo/worktrees", repo_path))?;

        let git_exclude_cmd = format!(
            "if [ -d \"{0}/.git\" ]; then mkdir -p \"{0}/.git/info\"; if ! grep -q \".demeteo/\" \"{0}/.git/info/exclude\" 2>/dev/null; then echo \".demeteo/\" >> \"{0}/.git/info/exclude\"; fi; fi",
            repo_path
        );
        let _ = local_run_command(&git_exclude_cmd);

        let worktree_add_cmd = format!(
            "git -C \"{}\" worktree add -b \"{}\" \"{}\"",
            repo_path, branch, sandbox_path
        );
        let output = local_run_command(&worktree_add_cmd)?;
        println!("[LocalSubprocessAdapter] Git Worktree provisioning output: {}", output);

        Ok(())
    }

    fn spawn_interactive(
        &self,
        _machine_id: &str,
        binary: &str,
        args: &[String],
        cwd: &str,
        env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        use crate::adapters::agent::acp::transport_local::LocalSubprocessTransport;
        LocalSubprocessTransport::spawn(binary, args, cwd, env)
            .map(|t| Box::new(t) as Box<dyn InteractiveHandle>)
    }
}
