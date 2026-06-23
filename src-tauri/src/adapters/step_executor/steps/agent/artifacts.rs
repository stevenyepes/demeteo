use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, compute_git_diff, read_worktree_file, resolve_declared_artifacts,
    WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn process_agent_artifacts(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        machine_str: &str,
        wt_path: &str,
        worktree_snapshot: &WorktreeSnapshot,
        worktree_base_ref: &Option<String>,
        produced_artifacts: &mut Vec<Artifact>,
    ) -> Result<(Option<String>, Vec<String>), String> {
        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());
        let decls = step_conf.artifacts.as_deref().unwrap_or(&[]);

        // 1. Process files using delta
        if !is_legacy {
            let always: Vec<&str> = decls
                .iter()
                .filter_map(|d| match &d.capture {
                    crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                        Some(path.as_str())
                    }
                    _ => None,
                })
                .collect();
            let changed = worktree_snapshot
                .delta(&*self.exec, machine_str, wt_path, &always, &[])
                .await;
            for rel_path in changed {
                let name = std::path::Path::new(&rel_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                if let Some(content) =
                    read_worktree_file(&*self.exec, machine_str, wt_path, &rel_path).await
                {
                    produced_artifacts.push(Artifact::tool_write(name, rel_path, content));
                }
            }
        }

        // 2. Compute git diff
        if !is_legacy {
            let diff_ref = worktree_base_ref.as_deref().unwrap_or("HEAD");
            let diff_body = compute_git_diff(&*self.exec, machine_str, wt_path, diff_ref).await;
            if !diff_body.trim().is_empty() {
                let diff_name = "code-diff".to_string();
                produced_artifacts.push(Artifact {
                    name: diff_name,
                    mime: "text/x-diff".into(),
                    content: diff_body,
                    source: crate::domain::artifact::ArtifactSource::Diff {
                        base: diff_ref.to_string(),
                        head: "WORKTREE".to_string(),
                        path_filter: None,
                    },
                });
            }
        }

        // 3. Commit changes
        let _ = commit_worktree_changes(
            &*self.exec,
            machine_str,
            wt_path,
            &format!("feat({}): {}", self.f_id.as_str(), step_conf.title),
        )
        .await;

        // 4. Resolve artifacts
        if !is_legacy {
            let refs = resolve_declared_artifacts(
                decls,
                produced_artifacts,
                &self.artifacts,
                &self.f_id_str,
                &step_exec.step_id.0,
            );
            let primary = if step_conf.kind == "parallel" {
                refs.iter()
                    .find(|r| r.contains("code-diff") || r.ends_with(".diff"))
                    .cloned()
                    .or_else(|| refs.first().cloned())
            } else {
                refs.first().cloned()
            };
            Ok((primary, refs))
        } else {
            Ok((None, vec![]))
        }
    }
}
