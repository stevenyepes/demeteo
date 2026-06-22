use rusqlite::params;

use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{Feature, StepExecution};
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};

use super::super::SqliteAdapter;

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::super::super::SqliteAdapter;
    use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
    use crate::domain::models::Project;
    use crate::domain::models::{Feature, StepExecution};
    use crate::ports::db::ProjectRepository;
    use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};

    fn setup() -> SqliteAdapter {
        let conn = Connection::open_in_memory().unwrap();
        SqliteAdapter::new(conn).unwrap()
    }

    fn make_feature(adapter: &SqliteAdapter, id: &str, project_id: &str) -> FeatureId {
        let fid = FeatureId::from(id.to_string());
        let pid = ProjectId::from(project_id.to_string());
        // Ensure the project exists (features.project_id FK).
        let _ = ProjectRepository::add(
            adapter,
            Project {
                id: pid.clone(),
                name: format!("project_{}", project_id),
                compute_type: "local".to_string(),
                remote_host: None,
                status: "idle".to_string(),
                nodes: 1,
                spend: 0.0,
                tokens: 0,
                created_at: 1000,
            },
        );
        FeatureRepository::add(
            adapter,
            Feature {
                id: fid.clone(),
                project_id: pid,
                workflow_id: None,
                title: "Test Feature".to_string(),
                status: "running".to_string(),
                total_cost: 0.0,
                tokens: 0,
                duration: "0s".to_string(),
                created_at: 1000,
                agent_kind: None,
                model: None,
                mr_url: None,
                mr_state: Some("none".to_string()),
            },
        )
        .unwrap();
        fid
    }

    fn make_step(
        adapter: &SqliteAdapter,
        id: &str,
        feature_id: &FeatureId,
        error_message: Option<&str>,
    ) -> StepExecutionId {
        let sid = StepExecutionId::from(id.to_string());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        adapter
            .step_create(StepExecution {
                id: sid.clone(),
                feature_id: feature_id.clone(),
                step_id: StepId::from("s1".to_string()),
                step_index: 0,
                step_kind: "agent".to_string(),
                status: "failed".to_string(),
                cost_usd: Some(1.0),
                tokens: Some(0),
                wall_clock_secs: Some(60),
                artifact_path: Some("/tmp/artifact".to_string()),
                artifact_paths: vec![],
                error_message: error_message.map(|s| s.to_string()),
                iteration_count: 0,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        sid
    }

    fn read_error(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<String> {
        adapter.step_get(sid).unwrap().unwrap().error_message
    }

    fn read_cost(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<f64> {
        adapter.step_get(sid).unwrap().unwrap().cost_usd
    }

    fn read_wall_clock(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<u64> {
        adapter.step_get(sid).unwrap().unwrap().wall_clock_secs
    }

    fn read_artifact(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Option<String> {
        adapter.step_get(sid).unwrap().unwrap().artifact_path
    }

    // ── StepExecutionPatch: error_message ────────────────────────────

    #[test]
    fn step_error_set_some_some() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f1", "p1");
        let sid = make_step(&adapter, "s1", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    error_message: Some(Some("new error".to_string())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_error(&adapter, &sid), Some("new error".to_string()));
    }

    #[test]
    fn step_error_clear_with_some_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f2", "p1");
        let sid = make_step(&adapter, "s2", &fid, Some("old error"));
        assert_eq!(read_error(&adapter, &sid), Some("old error".to_string()));
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    error_message: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_error(&adapter, &sid), None);
    }

    #[test]
    fn step_error_none_leaves_unchanged() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f3", "p1");
        let sid = make_step(&adapter, "s3", &fid, Some("persistent"));
        adapter
            .step_update(&sid, &StepExecutionPatch::default())
            .unwrap();
        assert_eq!(read_error(&adapter, &sid), Some("persistent".to_string()));
    }

    // ── StepExecutionPatch: cost_usd ─────────────────────────────────

    #[test]
    fn step_cost_set_some_some() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f4", "p1");
        let sid = make_step(&adapter, "s4", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    cost_usd: Some(Some(42.5)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_cost(&adapter, &sid), Some(42.5));
    }

    #[test]
    fn step_cost_clear_with_some_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f5", "p1");
        let sid = make_step(&adapter, "s5", &fid, None);
        assert_eq!(read_cost(&adapter, &sid), Some(1.0));
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    cost_usd: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_cost(&adapter, &sid), None);
    }

    #[test]
    fn step_cost_none_leaves_unchanged() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f6", "p1");
        let sid = make_step(&adapter, "s6", &fid, None);
        adapter
            .step_update(&sid, &StepExecutionPatch::default())
            .unwrap();
        assert_eq!(read_cost(&adapter, &sid), Some(1.0));
    }

    // ── StepExecutionPatch: wall_clock_secs ──────────────────────────

    #[test]
    fn step_wall_set_some_some() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f7", "p1");
        let sid = make_step(&adapter, "s7", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    wall_clock_secs: Some(Some(120)),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_wall_clock(&adapter, &sid), Some(120));
    }

    #[test]
    fn step_wall_clear_with_some_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f8", "p1");
        let sid = make_step(&adapter, "s8", &fid, None);
        assert_eq!(read_wall_clock(&adapter, &sid), Some(60));
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    wall_clock_secs: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_wall_clock(&adapter, &sid), None);
    }

    #[test]
    fn step_wall_none_leaves_unchanged() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f9", "p1");
        let sid = make_step(&adapter, "s9", &fid, None);
        adapter
            .step_update(&sid, &StepExecutionPatch::default())
            .unwrap();
        assert_eq!(read_wall_clock(&adapter, &sid), Some(60));
    }

    // ── StepExecutionPatch: artifact_path ────────────────────────────

    #[test]
    fn step_artifact_set_some_some() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f10", "p1");
        let sid = make_step(&adapter, "s10", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    artifact_path: Some(Some("/new/path".to_string())),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_artifact(&adapter, &sid), Some("/new/path".to_string()));
    }

    #[test]
    fn step_artifact_clear_with_some_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f11", "p1");
        let sid = make_step(&adapter, "s11", &fid, None);
        assert_eq!(
            read_artifact(&adapter, &sid),
            Some("/tmp/artifact".to_string())
        );
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    artifact_path: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(read_artifact(&adapter, &sid), None);
    }

    #[test]
    fn step_artifact_none_leaves_unchanged() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f12", "p1");
        let sid = make_step(&adapter, "s12", &fid, None);
        adapter
            .step_update(&sid, &StepExecutionPatch::default())
            .unwrap();
        assert_eq!(
            read_artifact(&adapter, &sid),
            Some("/tmp/artifact".to_string())
        );
    }

    // ── StepExecutionPatch: artifact_paths (V5 column) ──────────────

    fn read_artifact_paths(adapter: &SqliteAdapter, sid: &StepExecutionId) -> Vec<String> {
        adapter.step_get(sid).unwrap().unwrap().artifact_paths
    }

    #[test]
    fn step_artifact_paths_set_replaces_list_and_mirrors_legacy() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f100", "p1");
        let sid = make_step(&adapter, "s100", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    artifact_paths: Some(vec!["/a/x.md".to_string(), "/a/y.diff".to_string()]),
                    ..Default::default()
                },
            )
            .unwrap();
        let paths = read_artifact_paths(&adapter, &sid);
        assert_eq!(paths, vec!["/a/x.md".to_string(), "/a/y.diff".to_string()]);
        // Legacy single-path column mirrors the first entry.
        assert_eq!(read_artifact(&adapter, &sid), Some("/a/x.md".to_string()));
    }

    #[test]
    fn step_artifact_paths_clears_legacy_when_empty() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f101", "p1");
        let sid = make_step(&adapter, "s101", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    artifact_paths: Some(vec![]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(read_artifact_paths(&adapter, &sid).is_empty());
        assert_eq!(read_artifact(&adapter, &sid), None);
    }

    #[test]
    fn step_artifact_path_only_mirrors_into_list() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f102", "p1");
        let sid = make_step(&adapter, "s102", &fid, None);
        adapter
            .step_update(
                &sid,
                &StepExecutionPatch {
                    artifact_path: Some(Some("/legacy/only".to_string())),
                    ..Default::default()
                },
            )
            .unwrap();
        // The list itself isn't touched unless the caller sets it.
        // (Back-compat: legacy callers can keep writing the single
        // path and the new list stays empty.)
        assert!(read_artifact_paths(&adapter, &sid).is_empty());
        assert_eq!(
            read_artifact(&adapter, &sid),
            Some("/legacy/only".to_string())
        );
    }

    // ── FeaturePatch: total_cost/duration NOT NULL safety ────────────

    #[test]
    fn feature_update_status_preserves_cost_and_duration() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f13", "p1");
        FeatureRepository::update(
            &adapter,
            &fid,
            &FeaturePatch {
                status: Some("completed".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        let f = adapter.get(&fid).unwrap().unwrap();
        assert_eq!(f.status, "completed");
        // total_cost and duration must NOT have been set to NULL
        // (they have NOT NULL constraints in the schema).
        assert_eq!(f.total_cost, 0.0);
        assert_eq!(f.duration, "0s");
    }

    #[test]
    fn feature_update_cost_set_explicitly() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f14", "p1");
        FeatureRepository::update(
            &adapter,
            &fid,
            &FeaturePatch {
                total_cost: Some(Some(99.9)),
                ..Default::default()
            },
        )
        .unwrap();
        let f = adapter.get(&fid).unwrap().unwrap();
        assert_eq!(f.total_cost, 99.9);
    }

    #[test]
    fn feature_update_cost_skipped_with_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f15", "p1");
        FeatureRepository::update(
            &adapter,
            &fid,
            &FeaturePatch {
                total_cost: None,
                ..Default::default()
            },
        )
        .unwrap();
        let f = adapter.get(&fid).unwrap().unwrap();
        assert_eq!(f.total_cost, 0.0);
    }

    #[test]
    fn feature_update_cost_flattened_with_some_none() {
        let adapter = setup();
        let fid = make_feature(&adapter, "f16", "p1");
        // The current FeaturePatch::update uses .flatten() which
        // collapses Some(None) → None, so the column is left alone.
        FeatureRepository::update(
            &adapter,
            &fid,
            &FeaturePatch {
                total_cost: Some(None),
                ..Default::default()
            },
        )
        .unwrap();
        let f = adapter.get(&fid).unwrap().unwrap();
        assert_eq!(f.total_cost, 0.0);
    }
}

impl FeatureRepository for SqliteAdapter {
    fn get_active(&self, project_id: &ProjectId) -> Result<Vec<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state
                 FROM features WHERE project_id = ?1 AND status NOT IN ('archived', 'deleted') ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![project_id.0], |row| {
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    title: row.get(3)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }

    fn get(&self, id: &FeatureId) -> Result<Option<Feature>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state
                 FROM features WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                Ok(Feature {
                    id: row.get(0)?,
                    project_id: row.get(1)?,
                    workflow_id: row.get(2)?,
                    title: row.get(3)?,
                    status: row.get(4)?,
                    total_cost: row.get(5)?,
                    duration: row.get(6)?,
                    tokens: row.get(7)?,
                    created_at: row.get(8)?,
                    agent_kind: row.get(9)?,
                    model: row.get(10)?,
                    mr_url: row.get(11)?,
                    mr_state: row.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(f)) => Ok(Some(f)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn add(&self, f: Feature) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "INSERT INTO features (id, project_id, workflow_id, title, status, total_cost, duration, tokens, created_at, agent_kind, model, mr_url, mr_state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                f.id, f.project_id, f.workflow_id, f.title, f.status,
                f.total_cost, f.duration, f.tokens, f.created_at, f.agent_kind, f.model,
                f.mr_url, f.mr_state
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update(&self, id: &FeatureId, patch: &FeaturePatch) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let cost: Option<f64> = patch.total_cost.flatten();
        let dur: Option<String> = patch.duration.clone().flatten();
        let tokens: Option<i64> = patch.tokens.flatten();
        let agent_kind: Option<Option<String>> = patch.agent_kind.clone();
        let model: Option<Option<String>> = patch.model.clone();
        let mr_url: Option<Option<String>> = patch.mr_url.clone();
        let mr_state: Option<Option<String>> = patch.mr_state.clone();

        // Build the SET clause dynamically so a `None` field on the patch
        // actually means "leave the column alone". The previous code
        // always bound total_cost / duration when status was set, which
        // collapsed `None` → `NULL` and tripped the NOT NULL constraints
        // (see migration V1, features.total_cost / duration). step_retry
        // hit this because it intentionally preserves the existing cost
        // when re-running a failed step.
        let mut sets: Vec<&str> = Vec::new();
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(s) = &patch.status {
            sets.push("status=?");
            binds.push(Box::new(s.clone()));
        }
        if let Some(c) = cost {
            sets.push("total_cost=?");
            binds.push(Box::new(c));
        }
        if let Some(d) = &dur {
            sets.push("duration=?");
            binds.push(Box::new(d.clone()));
        }
        if let Some(t) = tokens {
            sets.push("tokens=?");
            binds.push(Box::new(t));
        }
        if let Some(ak) = agent_kind {
            sets.push("agent_kind=?");
            binds.push(Box::new(ak));
        }
        if let Some(m) = model {
            sets.push("model=?");
            binds.push(Box::new(m));
        }
        if let Some(url) = mr_url {
            sets.push("mr_url=?");
            binds.push(Box::new(url));
        }
        if let Some(state) = mr_state {
            sets.push("mr_state=?");
            binds.push(Box::new(state));
        }
        if sets.is_empty() {
            return Ok(());
        }
        let sql = format!("UPDATE features SET {} WHERE id=?", sets.join(", "));
        binds.push(Box::new(id.0.clone()));

        conn.execute(&sql, rusqlite::params_from_iter(binds.iter()))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn update_workflow_id(&self, id: &FeatureId, workflow_id: &WorkflowId) -> Result<(), String> {
        let conn = self.conn.lock()?;
        conn.execute(
            "UPDATE features SET workflow_id = ?2 WHERE id = ?1",
            params![id.0, workflow_id.0],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_create(&self, s: StepExecution) -> Result<(), String> {
        let conn = self.conn.lock()?;
        let artifact_paths_json =
            serde_json::to_string(&s.artifact_paths).map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO step_executions (id,feature_id,step_id,step_index,step_kind,status,cost_usd,tokens,wall_clock_secs,artifact_path,artifact_paths,error_message,iteration_count,created_at,updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                s.id, s.feature_id, s.step_id, s.step_index, s.step_kind, s.status,
                s.cost_usd, s.tokens, s.wall_clock_secs.map(|v| v as i64),
                s.artifact_path, artifact_paths_json, s.error_message, s.iteration_count,
                s.created_at, s.updated_at
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn step_get(&self, id: &StepExecutionId) -> Result<Option<StepExecution>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,tokens,wall_clock_secs,artifact_path,artifact_paths,error_message,iteration_count,created_at,updated_at
                 FROM step_executions WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;
        let mut iter = stmt
            .query_map(params![id.0], |row| {
                let artifact_paths_json: String = row.get(10)?;
                let artifact_paths: Vec<String> =
                    serde_json::from_str(&artifact_paths_json).unwrap_or_default();
                Ok(StepExecution {
                    id: row.get(0)?,
                    feature_id: row.get(1)?,
                    step_id: row.get(2)?,
                    step_index: row.get::<_, u32>(3)?,
                    step_kind: row.get(4)?,
                    status: row.get(5)?,
                    cost_usd: row.get(6)?,
                    tokens: row.get(7)?,
                    wall_clock_secs: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    artifact_path: row.get(9)?,
                    artifact_paths,
                    error_message: row.get(11)?,
                    iteration_count: row.get::<_, u32>(12)?,
                    created_at: row.get(13)?,
                    updated_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?;
        match iter.next() {
            Some(Ok(s)) => Ok(Some(s)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    }

    fn step_update(&self, id: &StepExecutionId, patch: &StepExecutionPatch) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let conn = self.conn.lock()?;
        // Build the SET clause dynamically so a `None` field on the patch
        // means "leave the column alone". Same pattern as FeaturePatch.
        let mut sets: Vec<&str> = Vec::new();
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(status) = &patch.status {
            sets.push("status=?");
            binds.push(Box::new(status.clone()));
        }
        match &patch.cost_usd {
            Some(Some(c)) => {
                sets.push("cost_usd=?");
                binds.push(Box::new(*c));
            }
            Some(None) => {
                sets.push("cost_usd=NULL");
            }
            None => {}
        }
        match &patch.tokens {
            Some(Some(t)) => {
                sets.push("tokens=?");
                binds.push(Box::new(*t));
            }
            Some(None) => {
                sets.push("tokens=NULL");
            }
            None => {}
        }
        match &patch.wall_clock_secs {
            Some(Some(w)) => {
                sets.push("wall_clock_secs=?");
                binds.push(Box::new(*w as i64));
            }
            Some(None) => {
                sets.push("wall_clock_secs=NULL");
            }
            None => {}
        }
        // `artifact_paths` (the canonical list) and `artifact_path` (the
        // legacy single-column projection) are written together so older
        // readers keep seeing a sensible primary. When the caller sets
        // `artifact_paths`, derive `artifact_path` from its first entry;
        // when the caller sets `artifact_path` only, mirror it into
        // `artifact_paths` so the next prompt render can find it via
        // the list.
        if let Some(paths) = &patch.artifact_paths {
            let json = serde_json::to_string(paths).map_err(|e| e.to_string())?;
            sets.push("artifact_paths=?");
            binds.push(Box::new(json));
            // Mirror the first entry into the legacy single-path column
            // unless the caller explicitly overrode it in the same patch.
            if patch.artifact_path.is_none() {
                let primary = paths.first().cloned();
                match primary {
                    Some(p) => {
                        sets.push("artifact_path=?");
                        binds.push(Box::new(p));
                    }
                    None => {
                        sets.push("artifact_path=NULL");
                    }
                }
            }
        }
        match &patch.artifact_path {
            Some(Some(a)) => {
                sets.push("artifact_path=?");
                binds.push(Box::new(a.clone()));
            }
            Some(None) => {
                sets.push("artifact_path=NULL");
            }
            None => {}
        }
        match &patch.error_message {
            Some(Some(e)) => {
                sets.push("error_message=?");
                binds.push(Box::new(e.clone()));
            }
            Some(None) => {
                sets.push("error_message=NULL");
            }
            None => {}
        }
        if let Some(i) = patch.iteration_count {
            sets.push("iteration_count=?");
            binds.push(Box::new(i));
        }
        if sets.is_empty() {
            return Ok(());
        }
        sets.push("updated_at=?");
        binds.push(Box::new(now));
        let sql = format!("UPDATE step_executions SET {} WHERE id=?", sets.join(", "));
        binds.push(Box::new(id.0.clone()));

        conn.execute(&sql, rusqlite::params_from_iter(binds.iter()))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn steps_for_feature(&self, feature_id: &FeatureId) -> Result<Vec<StepExecution>, String> {
        let conn = self.conn.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id,feature_id,step_id,step_index,step_kind,status,cost_usd,tokens,wall_clock_secs,artifact_path,artifact_paths,error_message,iteration_count,created_at,updated_at
                 FROM step_executions WHERE feature_id=?1 ORDER BY step_index ASC",
            )
            .map_err(|e| e.to_string())?;
        let iter = stmt
            .query_map(params![feature_id.0], |row| {
                let artifact_paths_json: String = row.get(10)?;
                let artifact_paths: Vec<String> =
                    serde_json::from_str(&artifact_paths_json).unwrap_or_default();
                Ok(StepExecution {
                    id: row.get(0)?,
                    feature_id: row.get(1)?,
                    step_id: row.get(2)?,
                    step_index: row.get::<_, u32>(3)?,
                    step_kind: row.get(4)?,
                    status: row.get(5)?,
                    cost_usd: row.get(6)?,
                    tokens: row.get(7)?,
                    wall_clock_secs: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    artifact_path: row.get(9)?,
                    artifact_paths,
                    error_message: row.get(11)?,
                    iteration_count: row.get::<_, u32>(12)?,
                    created_at: row.get(13)?,
                    updated_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut list = Vec::new();
        for r in iter {
            list.push(r.map_err(|e| e.to_string())?);
        }
        Ok(list)
    }
}
