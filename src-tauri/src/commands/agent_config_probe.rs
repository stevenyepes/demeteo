use tauri::State;
use crate::state::AppContext;
use crate::ports::agent_runtime::AgentContext;
use crate::domain::models::ConfigOptionValue;

/// Try to discover models via ACP `session/new` capability negotiation.
/// Returns `Ok` only if the agent exposes a `"model"` config option with options.
async fn probe_models_via_acp(
    ctx: &AppContext,
    machine_id: &str,
    agent_kind: &str,
) -> Result<Vec<ConfigOptionValue>, String> {
    let cwd = if machine_id == "local" || machine_id.is_empty() {
        std::env::var("HOME").unwrap_or_else(|_| ".".into())
    } else {
        ".".into()
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_thread_id = format!("probe-models-{}", now);
    let agent_ctx = AgentContext {
        thread_id: temp_thread_id.clone(),
        machine_id: machine_id.to_string(),
        binary: agent_kind.to_string(),
        args: vec![],
        env: crate::ports::agent_runtime::agent_base_env(),
        cwd,
        model: None,
        title: None,
        agent_exec: ctx.agent_exec.clone(),
        exec: ctx.exec.clone(),
    };

    let session = ctx
        .registry
        .get_or_spawn(&temp_thread_id, agent_kind, agent_ctx)
        .await
        .map_err(|e| format!("ACP probe spawn failed: {}", e))?;

    let info = session.session_info();
    let _ = session.kill();
    ctx.registry.kill(&temp_thread_id).await;

    if let Some(opts) = info.config_options {
        if let Some(opt) = opts.into_iter().find(|o| o.id == "model") {
            return Ok(opt.options);
        }
    }

    Err("No model config option in ACP session info".into())
}

/// Probe models by running `<binary> models` and parsing line-by-line output.
/// Each line is expected to be a model identifier (e.g. `provider/model`).
fn probe_models_via_cli(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    binary: &str,
) -> Result<Vec<ConfigOptionValue>, String> {
    let output = exec.run_command(machine_id, &format!("{} models", binary))?;
    let models: Vec<ConfigOptionValue> = output
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|model| ConfigOptionValue {
            value: model.to_string(),
            name: model.to_string(),
            description: None,
        })
        .collect();

    if models.is_empty() {
        return Err("CLI models command returned no output".into());
    }

    Ok(models)
}

fn fallback_models(agent_kind: &str) -> Vec<ConfigOptionValue> {
    match agent_kind {
        "claude-code" => vec![
            ConfigOptionValue {
                value: "claude-3-5-sonnet-latest".into(),
                name: "Claude 3.5 Sonnet (Latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "claude-3-5-haiku-latest".into(),
                name: "Claude 3.5 Haiku (Latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "claude-3-opus-latest".into(),
                name: "Claude 3 Opus (Latest)".into(),
                description: None,
            },
        ],
        "antigravity" => vec![
            ConfigOptionValue {
                value: "gemini-2.5-flash".into(),
                name: "Gemini 2.5 Flash".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "gemini-2.5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "gemini-1.5-pro".into(),
                name: "Gemini 1.5 Pro".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "gemini-1.5-flash".into(),
                name: "Gemini 1.5 Flash".into(),
                description: None,
            },
        ],
        "opencode" | "hermes" => vec![
            ConfigOptionValue {
                value: "anthropic/claude-3-5-sonnet-20241022".into(),
                name: "Claude 3.5 Sonnet (Latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "openai/gpt-4o".into(),
                name: "GPT-4o".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "google/gemini-2.5-flash".into(),
                name: "Gemini 2.5 Flash".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "deepseek/deepseek-coder-v2".into(),
                name: "DeepSeek Coder V2".into(),
                description: None,
            },
        ],
        _ => vec![],
    }
}

#[tauri::command]
pub async fn get_agent_models(
    ctx: State<'_, AppContext>,
    machine_id: String,
    agent_kind: String,
) -> Result<Vec<ConfigOptionValue>, String> {
    // 1. Try ACP session/new probe — currently no agent runtime populates
    //    config_options, but future ACP-capable agents may.
    if let Ok(models) = probe_models_via_acp(&ctx, &machine_id, &agent_kind).await {
        return Ok(models);
    }

    // 2. CLI model probing for agents that expose a `models` subcommand
    if agent_kind == "opencode" || agent_kind == "hermes" {
        if let Ok(models) = probe_models_via_cli(ctx.exec.as_ref(), &machine_id, &agent_kind) {
            return Ok(models);
        }
    }

    // 3. Fallback to hardcoded lists when dynamic probing is unavailable
    Ok(fallback_models(&agent_kind))
}
