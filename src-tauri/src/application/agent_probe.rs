use crate::domain::models::ConfigOptionValue;
use crate::ports::agent_runtime::AgentContext;
use crate::state::AppContext;

pub async fn discover_models(
    ctx: &AppContext,
    machine_id: String,
    agent_kind: String,
) -> Result<Vec<ConfigOptionValue>, String> {
    // 1. Try ACP session/new probe
    if let Ok(models) = probe_models_via_acp(ctx, &machine_id, &agent_kind).await {
        return Ok(models);
    }

    // 2. CLI model probing for agents that expose a `models` subcommand
    if agent_kind == "opencode" || agent_kind == "hermes" || agent_kind == "antigravity" {
        if let Ok(models) = probe_models_via_cli(ctx.exec.as_ref(), &machine_id, &agent_kind).await
        {
            return Ok(models);
        }
    }

    // 3. Fallback to hardcoded lists when dynamic probing is unavailable
    Ok(fallback_models(&agent_kind))
}

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
    let probe_binary = ctx
        .registry
        .runtime_for(agent_kind)
        .map(|r| r.binary().to_string())
        .unwrap_or_else(|| agent_kind.to_string());
    let agent_ctx = AgentContext {
        thread_id: temp_thread_id.clone(),
        machine_id: machine_id.to_string(),
        binary: probe_binary,
        args: vec![],
        env: crate::ports::agent_runtime::agent_base_env(),
        cwd,
        model: None,
        title: None,
        agent_exec: ctx.agent_exec.clone(),
        exec: ctx.exec.clone(),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: false,
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

async fn probe_models_via_cli(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    agent_kind: &str,
) -> Result<Vec<ConfigOptionValue>, String> {
    // NOTE: do NOT add a "claude-code" arm here. The `claude` CLI has no
    // `models` subcommand — `claude models` would be parsed as a *prompt*
    // ("models") and start a session instead of listing anything. claude-code
    // models come from the alias fallback in `fallback_models` instead, and
    // `discover_models` deliberately excludes claude-code from this CLI path.
    let binary = match agent_kind {
        "antigravity" => "agy",
        other => other,
    };
    let output = exec
        .run_command(machine_id, &format!("{} models", binary))
        .await?;
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
        // The `claude` CLI has no model-listing command (unlike `opencode
        // models`), so there is nothing to probe. What it *does* expose is
        // `--model`, which accepts an alias for the latest model
        // ('opus'/'sonnet'/'haiku'/'fable') or a full model id. We store the
        // aliases here: they're passed straight through to `--model` and the
        // CLI resolves them to the current generation at runtime, so this list
        // never goes stale. Users wanting a pinned build can type a full id in
        // the custom-override field of the model picker.
        "claude-code" => vec![
            ConfigOptionValue {
                value: "opus".into(),
                name: "Claude Opus (latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "sonnet".into(),
                name: "Claude Sonnet (latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "haiku".into(),
                name: "Claude Haiku (latest)".into(),
                description: None,
            },
            ConfigOptionValue {
                value: "fable".into(),
                name: "Claude Fable (latest)".into(),
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
