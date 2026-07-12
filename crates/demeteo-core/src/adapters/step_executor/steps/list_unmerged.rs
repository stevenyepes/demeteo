use crate::paths;

pub(crate) struct ConflictFile {
    pub(crate) path: String,
    pub(crate) kind: String,
}

pub(crate) async fn list_unmerged_files(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec
        .run_command(
            machine_id,
            &format!(
                "git -C {} status --porcelain --untracked-files=no",
                paths::shell_escape_posix(repo_dir)
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}
