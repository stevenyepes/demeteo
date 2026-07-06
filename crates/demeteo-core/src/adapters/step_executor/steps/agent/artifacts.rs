use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, compute_git_diff, is_under_prefix, read_worktree_file,
    resolve_declared_artifacts, MissingArtifact, WorktreeSnapshot,
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
    ) -> Result<(Option<String>, Vec<String>, Vec<MissingArtifact>), String> {
        let decls = step_conf.artifacts.as_deref().unwrap_or(&[]);

        // 1. Process files using delta
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
        // Snapshot the subset of `changed` that sits OUTSIDE the
        // artifact subdir — these are the paths the user actually
        // asked the agent to create or modify. We capture them
        // before consuming `changed` in the loop below so they can
        // be forwarded to `commit_worktree_changes`'s guard log.
        let trimmed_subdir = self
            .artifact_subdir
            .trim()
            .trim_start_matches("./")
            .trim_end_matches('/');
        let non_artifact_writes: Vec<String> = changed
            .iter()
            .filter(|p| !is_under_prefix(p, trimmed_subdir))
            .cloned()
            .collect();
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

        // 2. Compute git diff. Prefer the feature's fork point (where
        // `branch_name` diverged from the default branch) over
        // `worktree_base_ref` (this attempt's pre-run tip) so the review
        // diff always covers the complete feature change, not just the
        // latest retry's incremental fix — see `resolve_fork_point_ref`.
        // `worktree_base_ref` remains the fallback (fork point
        // unavailable, e.g. default branch not configured) and is still
        // used unchanged by the no-op-commit guard in `handle_agent_step`.
        let fork_point = self.resolve_fork_point_ref(machine_str).await;
        let diff_ref = fork_point
            .as_deref()
            .or(worktree_base_ref.as_deref())
            .unwrap_or("HEAD")
            .to_string();
        let diff_body = compute_git_diff(&*self.exec, machine_str, wt_path, &diff_ref).await;
        if !diff_body.trim().is_empty() {
            produced_artifacts.push(Artifact {
                name: "code-diff".to_string(),
                mime: "text/x-diff".into(),
                content: diff_body,
                source: crate::domain::artifact::ArtifactSource::Diff {
                    base: diff_ref.clone(),
                    head: "WORKTREE".to_string(),
                    path_filter: None,
                },
            });
        }

        // 3. Commit changes. The `non_artifact_writes` list computed
        // above (paths the agent wrote that are NOT under the
        // artifact subdir) is passed so the guard log inside
        // `commit_worktree_changes` can flag the historical "agent
        // put the real deliverable under artifacts/ instead of at
        // the real path" failure mode (see declared.rs).
        let _ = commit_worktree_changes(
            &*self.exec,
            machine_str,
            wt_path,
            &format!(
                "feat({}): {}",
                self.f_id.as_str(),
                step_conf.title.to_lowercase()
            ),
            &self.artifact_subdir,
            self.commit_artifacts,
            &non_artifact_writes,
        )
        .await;

        // 4. Resolve artifacts. `missing` is the set of declared
        // `ByName`/`LastWriteTo` deliverables the agent never produced;
        // the caller fails the step on a non-empty list so a step that
        // "ran" but produced no plan/spec/report is visible instead of a
        // green step with an empty artifact.
        let (refs, missing) = resolve_declared_artifacts(
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
        Ok((primary, refs, missing))
    }
}
