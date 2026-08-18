use rusqlite::params;

use crate::domain::ids::{ProjectId, WorkflowId};
use crate::domain::models::{
    EffortLevel, Project, ProjectSettings, ProjectWorkflowOverride, Repository, WorktreeStrategy,
};
use crate::ports::db::ProjectRepository;

use super::super::SqliteAdapter;

/// Read an `effort` / `default_effort` column. An unknown/stale string
/// degrades to `None` (inherit) rather than failing the row.
fn effort_from_row(row: &rusqlite::Row, idx: usize) -> rusqlite::Result<Option<EffortLevel>> {
    let raw: Option<String> = row.get(idx)?;
    Ok(raw.as_deref().and_then(EffortLevel::parse))
}

/// The two shapes the `project_settings.harnesses` TEXT column (V8) is allowed
/// to hold. Both are harness *configuration*, which is why they share one
/// column instead of costing a migration for a `Vec<String>`:
///
/// * `{"lint": "npm run lint"}` — the original map, and still what is written
///   whenever no validation gates are selected. Every row written before HB5
///   has this shape, and so does every row written after it by a project that
///   never ticked a gate, so the column's content is unchanged for them.
/// * `{"harnesses": {...}, "validation_gates": ["lint"]}` — written only once a
///   selection exists.
///
/// Order matters, and the *map* has to be tried first. Every field of the
/// envelope is optional, so an untagged match against it would accept any JSON
/// object at all — including a legacy map, whose entries it would silently
/// discard as unknown fields. The reverse cannot happen: the envelope's
/// `harnesses` key holds an object, and a map's values are all strings, so a
/// real envelope can never parse as a map.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum HarnessesColumn {
    Map(std::collections::HashMap<String, String>),
    Envelope {
        harnesses: Option<std::collections::HashMap<String, String>>,
        #[serde(default)]
        validation_gates: Option<Vec<String>>,
    },
}

/// Split the stored column into the two `WorktreeStrategy` fields it carries.
/// An unparseable column degrades to "nothing configured", exactly as the
/// previous `from_str(&s).ok()` did.
fn harnesses_from_column(
    raw: Option<String>,
) -> (
    Option<std::collections::HashMap<String, String>>,
    Option<Vec<String>>,
) {
    match raw.and_then(|s| serde_json::from_str::<HarnessesColumn>(&s).ok()) {
        Some(HarnessesColumn::Envelope {
            harnesses,
            validation_gates,
        }) => (harnesses, validation_gates),
        Some(HarnessesColumn::Map(map)) => (Some(map), None),
        None => (None, None),
    }
}

/// Render the column. Stays in the legacy bare-map shape unless a gate
/// selection exists, so a project that never uses HB6's checkbox writes
/// byte-identical rows to the ones it wrote before.
fn harnesses_to_column(strategy: &WorktreeStrategy) -> Option<String> {
    let gates = strategy
        .validation_gates
        .as_ref()
        .filter(|g| !g.is_empty())
        .cloned();
    match gates {
        None => strategy
            .harnesses
            .as_ref()
            .and_then(|h| serde_json::to_string(h).ok()),
        Some(gates) => serde_json::to_string(&HarnessesColumn::Envelope {
            harnesses: strategy.harnesses.clone(),
            validation_gates: Some(gates),
        })
        .ok(),
    }
}

impl ProjectRepository for SqliteAdapter {
    fn get_projects(&self) -> Result<Vec<Project>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, compute_type, remote_host, status,
                        ((SELECT COUNT(*) FROM step_executions se JOIN features f ON se.feature_id = f.id WHERE f.project_id = projects.id AND se.status = 'running' AND se.step_kind = 'agent') + (SELECT COUNT(*) FROM subtask_runs sr JOIN features f ON sr.feature_id = f.id WHERE f.project_id = projects.id AND sr.status = 'running')) AS nodes,
                        spend,
                        COALESCE((SELECT SUM(tokens) FROM features WHERE project_id = projects.id), 0) AS tokens,
                        created_at
                 FROM projects ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map([], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    compute_type: row.get(2)?,
                    remote_host: row.get(3)?,
                    status: row.get(4)?,
                    nodes: row.get(5)?,
                    spend: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn get_project(&self, id: &ProjectId) -> Result<Option<Project>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, compute_type, remote_host, status,
                        ((SELECT COUNT(*) FROM step_executions se JOIN features f ON se.feature_id = f.id WHERE f.project_id = projects.id AND se.status = 'running' AND se.step_kind = 'agent') + (SELECT COUNT(*) FROM subtask_runs sr JOIN features f ON sr.feature_id = f.id WHERE f.project_id = projects.id AND sr.status = 'running')) AS nodes,
                        spend,
                        COALESCE((SELECT SUM(tokens) FROM features WHERE project_id = projects.id), 0) AS tokens,
                        created_at
                 FROM projects WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                Ok(Project {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    compute_type: row.get(2)?,
                    remote_host: row.get(3)?,
                    status: row.get(4)?,
                    nodes: row.get(5)?,
                    spend: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(p)) => Ok(Some(p)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn add(&self, p: Project) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO projects (id, name, compute_type, remote_host, status, nodes, spend, tokens, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![p.id, p.name, p.compute_type, p.remote_host, p.status, p.nodes, p.spend, p.tokens, p.created_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, p: Project) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE projects SET name = ?2, compute_type = ?3, remote_host = ?4,
             status = ?5, nodes = ?6, tokens = ?7 WHERE id = ?1",
            params![
                p.id,
                p.name,
                p.compute_type,
                p.remote_host,
                p.status,
                p.nodes,
                p.tokens
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_status(&self, id: &ProjectId, status: &str) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE projects SET status = ?2 WHERE id = ?1",
            params![id.0, status],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete(&self, id: &ProjectId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute("DELETE FROM projects WHERE id = ?1", params![id.0])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn delete_repositories_for(&self, project_id: &ProjectId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "DELETE FROM repositories WHERE project_id = ?1",
            params![project_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn add_repository(&self, repo: Repository) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO repositories (id, project_id, provider_id, repo_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![repo.id, repo.project_id, repo.provider_id, repo.repo_path],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn get_repositories_for(&self, project_id: &ProjectId) -> Result<Vec<Repository>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, provider_id, repo_path
                 FROM repositories WHERE project_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], |row| {
                Ok(Repository {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    provider_id: row.get(2)?,
                    repo_path: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn get_settings(&self, project_id: &ProjectId) -> Result<Option<ProjectSettings>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT project_id, default_branch, branch_prefix, test_command, pr_template,
                        conflict_policy, feature_lifecycle, build_command, coverage_command,
                        conventions_file, default_agent_kind, default_model, harnesses,
                        artifact_subdir, commit_artifacts, default_loop_iterations,
                        extra_writable_paths, prepare_command, default_effort,
                        default_max_budget_usd, default_workflow_id, review_entrypoint,
                        sync_resolver_agent_kind, sync_resolver_model, sync_resolver_effort
                 FROM project_settings WHERE project_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![project_id.0], |row| {
                let (harnesses, validation_gates) = harnesses_from_column(row.get(12)?);
                let commit_artifacts: i64 = row.get(14)?;
                let default_loop_iterations: Option<i64> = row.get(15)?;
                let extra_writable_paths_json: Option<String> = row.get(16)?;
                let prepare_command: Option<String> = row.get(17)?;
                Ok(ProjectSettings {
                    project_id: row.get(0)?,
                    worktree_strategy: WorktreeStrategy {
                        default_branch: row.get(1)?,
                        branch_prefix: row.get(2)?,
                        test_command: row.get(3)?,
                        build_command: row.get(7)?,
                        coverage_command: row.get(8)?,
                        conventions_file: row.get(9)?,
                        pr_template: row.get(4)?,
                        harnesses,
                        validation_gates,
                        prepare_command,
                        extra_writable_paths: extra_writable_paths_json
                            .and_then(|s| serde_json::from_str(&s).ok())
                            .unwrap_or_default(),
                    },
                    conflict_policy: row.get(5)?,
                    feature_lifecycle: row.get(6)?,
                    default_agent_kind: row.get(10)?,
                    default_model: row.get(11)?,
                    default_effort: effort_from_row(row, 18)?,
                    default_workflow_id: row.get(20)?,
                    artifact_subdir: row.get(13)?,
                    commit_artifacts: commit_artifacts != 0,
                    default_loop_iterations: default_loop_iterations.map(|v| v as u32),
                    default_max_budget_usd: row.get(19)?,
                    review_entrypoint: row.get(21)?,
                    sync_resolver_agent_kind: row.get(22)?,
                    sync_resolver_model: row.get(23)?,
                    sync_resolver_effort: effort_from_row(row, 24)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn save_settings(&self, s: ProjectSettings) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let harnesses_json = harnesses_to_column(&s.worktree_strategy);
        let extra_writable_paths_json = if s.worktree_strategy.extra_writable_paths.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&s.worktree_strategy.extra_writable_paths)
                    .map_err(|e| e.to_string())?,
            )
        };
        conn.execute(
            "INSERT OR REPLACE INTO project_settings
             (project_id, default_branch, branch_prefix, test_command, build_command,
              coverage_command, conventions_file, pr_template, conflict_policy, feature_lifecycle,
              default_agent_kind, default_model, harnesses, artifact_subdir, commit_artifacts,
              default_loop_iterations, extra_writable_paths, prepare_command, default_effort,
              default_max_budget_usd, default_workflow_id, review_entrypoint,
              sync_resolver_agent_kind, sync_resolver_model, sync_resolver_effort)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            params![
                s.project_id,
                s.worktree_strategy.default_branch,
                s.worktree_strategy.branch_prefix,
                s.worktree_strategy.test_command,
                s.worktree_strategy.build_command,
                s.worktree_strategy.coverage_command,
                s.worktree_strategy.conventions_file,
                s.worktree_strategy.pr_template,
                s.conflict_policy,
                s.feature_lifecycle,
                s.default_agent_kind,
                s.default_model,
                harnesses_json,
                s.artifact_subdir,
                s.commit_artifacts as i64,
                s.default_loop_iterations.map(|v| v as i64),
                extra_writable_paths_json,
                s.worktree_strategy.prepare_command,
                s.default_effort.map(|e| e.as_str()),
                s.default_max_budget_usd,
                s.default_workflow_id,
                s.review_entrypoint,
                s.sync_resolver_agent_kind,
                s.sync_resolver_model,
                s.sync_resolver_effort.map(|e| e.as_str()),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn list_workflow_overrides(
        &self,
        project_id: &ProjectId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT project_id, workflow_id, step_id, agent_kind, model, effort
                 FROM project_workflow_overrides WHERE project_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], row_to_override)
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn list_overrides_for_workflow(
        &self,
        project_id: &ProjectId,
        workflow_id: &WorkflowId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT project_id, workflow_id, step_id, agent_kind, model, effort
                 FROM project_workflow_overrides
                 WHERE project_id = ?1 AND workflow_id = ?2",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0, workflow_id.0], row_to_override)
            .map_err(|e| e.to_string())?;
        iter.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    fn upsert_workflow_override(&self, ov: ProjectWorkflowOverride) -> Result<(), String> {
        let conn = self.conn.lock()?;
        // Persisted discriminator: workflow-level rows store step_id = ''.
        let step_key = ov.step_id.clone().unwrap_or_default();
        // A row that overrides no field at all is a no-op overlay — store it as
        // "no override" by deleting any existing row instead of persisting an
        // all-NULL row the resolver would have to special-case anyway. Every
        // overridable dimension must be in this predicate: an effort-only row
        // is a real override, not an empty one.
        if ov.agent_kind.is_none() && ov.model.is_none() && ov.effort.is_none() {
            conn.execute(
                "DELETE FROM project_workflow_overrides
                 WHERE project_id = ?1 AND workflow_id = ?2 AND step_id = ?3",
                params![ov.project_id.0, ov.workflow_id.0, step_key],
            )
            .map_err(|e| e.to_string())?;
            return Ok(());
        }
        conn.execute(
            "INSERT OR REPLACE INTO project_workflow_overrides
             (project_id, workflow_id, step_id, agent_kind, model, effort)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ov.project_id.0,
                ov.workflow_id.0,
                step_key,
                ov.agent_kind,
                ov.model,
                ov.effort.map(|e| e.as_str())
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Map a `(project_id, workflow_id, step_id, agent_kind, model, effort)` row to
/// a `ProjectWorkflowOverride`, normalising the empty-string step discriminator
/// back to `None` (workflow-level).
fn row_to_override(row: &rusqlite::Row) -> rusqlite::Result<ProjectWorkflowOverride> {
    let step_id: String = row.get(2)?;
    Ok(ProjectWorkflowOverride {
        project_id: row.get(0)?,
        workflow_id: row.get(1)?,
        step_id: if step_id.is_empty() {
            None
        } else {
            Some(step_id)
        },
        agent_kind: row.get(3)?,
        model: row.get(4)?,
        effort: effort_from_row(row, 5)?,
    })
}
