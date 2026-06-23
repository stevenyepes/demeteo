use rusqlite::params;

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::StepExecution;
use crate::ports::db::StepExecutionPatch;

use super::super::SqliteAdapter;

pub fn step_create(adapter: &SqliteAdapter, s: StepExecution) -> Result<(), String> {
    let conn = adapter.conn.lock()?;
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

pub fn step_get(
    adapter: &SqliteAdapter,
    id: &StepExecutionId,
) -> Result<Option<StepExecution>, String> {
    let conn = adapter.conn.lock()?;
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

pub fn step_update(
    adapter: &SqliteAdapter,
    id: &StepExecutionId,
    patch: &StepExecutionPatch,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let conn = adapter.conn.lock()?;
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
    if let Some(paths) = &patch.artifact_paths {
        let json = serde_json::to_string(paths).map_err(|e| e.to_string())?;
        sets.push("artifact_paths=?");
        binds.push(Box::new(json));
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

pub fn steps_for_feature(
    adapter: &SqliteAdapter,
    feature_id: &FeatureId,
) -> Result<Vec<StepExecution>, String> {
    let conn = adapter.conn.lock()?;
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

#[cfg(test)]
#[path = "../../../../tests/infrastructure/database/feature_steps.rs"]
mod tests;
