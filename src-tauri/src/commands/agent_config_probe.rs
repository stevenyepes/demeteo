use std::collections::HashMap;
use tauri::State;
use crate::state::AppContext;
use crate::ports::agent_runtime::AgentContext;
use crate::domain::models::ConfigOptionValue;

#[tauri::command]
pub async fn get_agent_models(
    ctx: State<'_, AppContext>,
    machine_id: String,
    agent_kind: String,
) -> Result<Vec<ConfigOptionValue>, String> {
    // 1. Resolve safe CWD for spawning probe
    let cwd = if machine_id == "local" || machine_id.is_empty() {
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    } else {
        ".".into()
    };
    
    // 2. Build AgentContext
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_thread_id = format!("probe-models-{}", now);
    let agent_ctx = AgentContext {
        thread_id: temp_thread_id.clone(),
        machine_id: machine_id.clone(),
        binary: agent_kind.clone(),
        args: vec!["acp".to_string()],
        env: HashMap::new(),
        cwd,
        agent_exec: ctx.agent_exec.clone(),
        exec: ctx.exec.clone(),
    };

    // 3. Spawns session using registry. This completes initialize & session/new.
    let session = ctx
        .registry
        .get_or_spawn(&temp_thread_id, &agent_kind, agent_ctx)
        .await
        .map_err(|e| format!("Failed to spawn agent for model probe: {}", e))?;

    // 4. Retrieve session config options
    let info = session.session_info();

    // 5. Clean up the spawned agent process BEFORE the registry drops
    // the session Arc. `registry.kill` only removes the map entry; the
    // session's `JsonRpcClient` background reader thread keeps the
    // transport Arc alive until the reader thread sees EOF. We must
    // explicitly call `session.kill()` to close the SSH channel (which
    // sends SIGHUP to the remote `opencode acp`) so the remote process
    // is reaped and we don't leak agents on the server.
    let _ = session.kill();
    ctx.registry.kill(&temp_thread_id).await;
    // `session` Arc drops at end of function; JsonRpcClient Drop is now
    // a no-op because we already closed the transport above.

    if let Some(opts) = info.config_options {
        if let Some(opt) = opts.into_iter().find(|o| o.id == "model") {
            return Ok(opt.options);
        }
    }
    
    Ok(vec![])
}
