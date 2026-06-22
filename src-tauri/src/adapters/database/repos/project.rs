use rusqlite::params;

use crate::domain::ids::ProjectId;
use crate::domain::models::{Project, ProjectSettings, Repository, WorktreeStrategy};
use crate::ports::db::ProjectRepository;

use super::super::SqliteAdapter;

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
                        conventions_file, default_agent_kind, default_model, harnesses
                 FROM project_settings WHERE project_id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![project_id.0], |row| {
                let harnesses: Option<String> = row.get(12)?;
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
                        harnesses: harnesses.and_then(|s| serde_json::from_str(&s).ok()),
                    },
                    conflict_policy: row.get(5)?,
                    feature_lifecycle: row.get(6)?,
                    default_agent_kind: row.get(10)?,
                    default_model: row.get(11)?,
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
        let harnesses_json = s
            .worktree_strategy
            .harnesses
            .as_ref()
            .and_then(|h| serde_json::to_string(h).ok());
        conn.execute(
            "INSERT OR REPLACE INTO project_settings
             (project_id, default_branch, branch_prefix, test_command, build_command,
              coverage_command, conventions_file, pr_template, conflict_policy, feature_lifecycle,
              default_agent_kind, default_model, harnesses)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                harnesses_json
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}
