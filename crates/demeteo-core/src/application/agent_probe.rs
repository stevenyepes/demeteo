use crate::domain::models::ConfigOptionValue;
use crate::ports::agent_runtime::AgentContext;
use crate::state::AppContext;

pub async fn discover_models(
    ctx: &AppContext,
    machine_id: String,
    agent_kind: String,
) -> Result<Vec<ConfigOptionValue>, String> {
    // codex exposes neither ACP nor a `models` subcommand; its active provider
    // and model live in ~/.codex/config.toml. Resolve it up front so (a) we
    // don't spawn a pointless ACP probe, and (b) a user who repointed codex at
    // a custom provider (MiniMax, a local gateway, ...) is offered *that*
    // provider's model rather than OpenAI slugs the endpoint rejects
    // (MiniMax answers `gpt-5.3-codex` with `unknown model`, code 2013).
    if agent_kind == "codex" {
        return Ok(discover_codex_models(ctx.exec.as_ref(), &machine_id).await);
    }

    // 1. Try ACP session/new probe
    if let Ok(models) = probe_models_via_acp(ctx, &machine_id, &agent_kind).await {
        return Ok(models);
    }

    // 2. CLI model probing for agents that declare a `models` subcommand.
    let lists_models = ctx
        .registry
        .runtime_for(&agent_kind)
        .map(|r| r.capabilities().lists_models)
        .unwrap_or(false);
    if lists_models {
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
        env: crate::ports::agent_runtime::agent_base_env(ctx.exec.as_ref(), machine_id).await,
        cwd,
        model: None,
        effort: None,
        title: None,
        agent_exec: ctx.agent_exec.clone(),
        exec: ctx.exec.clone(),
        permissions: crate::domain::permission::PermissionProfile::all_allow(),
        bare_mode: false,
        tool_allowlist: None,
        max_turns: None,
        // Model-probe session: no budget guardrail.
        max_budget_usd: None,
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
    // Every kind reaching this CLI-listing path has a binary name equal to
    // its kind (opencode, hermes); claude-code is excluded above.
    let binary = agent_kind;
    // Interactive login shell so a tool-manager-provided binary (mise/asdf/
    // nvm) is on PATH — matching the availability probe and agent spawn.
    // A plain `run_command` runs non-login and can't find e.g. a
    // mise-managed `opencode`, so `opencode models` errors and the caller
    // silently falls back to the hardcoded `fallback_models` list. See
    // `ShellOptions::interactive`.
    let output = exec
        .run_command_with(
            machine_id,
            &format!("{} models", binary),
            crate::ports::execution::ShellOptions::login_interactive(),
        )
        .await?;
    let models: Vec<ConfigOptionValue> = output
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|model| {
            let supports_images = model_supports_images_by_name(agent_kind, model);
            ConfigOptionValue {
                value: model.to_string(),
                name: model.to_string(),
                description: None,
                supports_images,
            }
        })
        .collect();

    if models.is_empty() {
        return Err("CLI models command returned no output".into());
    }

    Ok(models)
}

/// Best-effort vision-capability guess for a model string the runtime
/// reports but that has no row in the bundled fallback table.
///
/// Substring rules (case-insensitive):
///   * positive — `gpt-4`, `gemini`, `claude`, `vision`, `opus`,
///     `sonnet`, `haiku`, `minimax`
///   * negative — `embedding`, `whisper` (overrides positives)
///
/// Unknown strings return `false` so the UI shows a soft warning
/// instead of silently dropping the image. This is the
/// pessimistic path: only models whose name pattern is *known*
/// to imply vision support are flagged true. The `minimax` token
/// covers vendor vision-capable models such as
/// `minimax-coding-plan/MiniMax-M3` — keep this list in sync with
/// `src/lib/modelImageSupport.ts` so the frontend warning banner
/// and the orchestrator's runtime probe agree.
pub fn model_supports_images_by_name(_agent_kind: &str, model: &str) -> bool {
    let m = model.trim().to_lowercase();
    if m.is_empty() {
        return false;
    }
    // Negatives override positives — `text-embedding-ada-002` must
    // not be flagged as vision-capable just because it contains
    // nothing that the positives would hit, but a hypothetical
    // `embedding-vision-experimental` should still take the negative.
    if m.contains("embedding") || m.contains("whisper") {
        return false;
    }
    const POSITIVES: &[&str] = &[
        "gpt-4", "gpt-5", "gemini", "claude", "vision", "opus", "sonnet", "haiku", "fable",
        "minimax",
    ];
    POSITIVES.iter().any(|needle| m.contains(needle))
}

/// Resolve codex's model list from the user's `~/.codex/config.toml`.
///
/// codex has no `models` subcommand and no ACP session to probe, so its model
/// menu is driven by the configured provider. When a *custom* provider is
/// active (anything other than the built-in `openai`), the bundled OpenAI slug
/// list in [`fallback_models`] would be sent to an endpoint that doesn't know
/// those slugs and hard-fails the turn — so instead we surface the configured
/// default model, and rely on the picker's custom-override field for anything
/// else the provider exposes. With no custom provider (OpenAI, or no config at
/// all) we return the bundled catalog-backed slug list unchanged.
async fn discover_codex_models(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
) -> Vec<ConfigOptionValue> {
    // `cat` (not `read_file`) so the shell expands `~` uniformly on local and
    // remote machines; a missing file yields empty output → OpenAI defaults.
    let toml = exec
        .run_command(machine_id, "cat ~/.codex/config.toml 2>/dev/null")
        .await
        .unwrap_or_default();
    let (provider, model) = parse_codex_config(&toml);

    let is_custom_provider = provider
        .as_deref()
        .is_some_and(|p| !p.is_empty() && p != "openai");
    if !is_custom_provider {
        return fallback_models("codex");
    }

    // Custom provider: OpenAI slugs 404. Offer the configured default model if
    // there is one; otherwise leave the menu empty so the user types their
    // provider's slug into the custom-override field.
    match model.filter(|m| !m.is_empty()) {
        Some(m) => vec![ConfigOptionValue {
            supports_images: model_supports_images_by_name("codex", &m),
            description: provider.map(|p| format!("~/.codex/config.toml ({p})")),
            value: m.clone(),
            name: m,
        }],
        None => vec![],
    }
}

/// Extract the top-level `model_provider` and `model` string values from a
/// codex `config.toml`. Only keys *before the first `[section]` header* are
/// read: codex's process-level defaults live at the top, while `[section]`
/// bodies carry provider-scoped keys (e.g. `[model_providers.minimax]`) that
/// must not be mistaken for the active selection. Returns
/// `(model_provider, model)`.
fn parse_codex_config(toml: &str) -> (Option<String>, Option<String>) {
    let mut provider = None;
    let mut model = None;
    for raw in toml.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            break; // top-level keys only.
        }
        if line.starts_with('#') {
            continue;
        }
        // `model_provider` must be tried before `model`: the latter's key is a
        // prefix of the former, but `toml_str_value` anchors on the `=` so a
        // `model_provider = ...` line never matches the `model` key.
        if let Some(v) = toml_str_value(line, "model_provider") {
            provider = Some(v);
        } else if let Some(v) = toml_str_value(line, "model") {
            model = Some(v);
        }
    }
    (provider, model)
}

/// Parse a single `key = "value"` TOML line for an exact `key`, tolerating
/// single or double quotes and surrounding whitespace. Returns `None` if the
/// line isn't that key or isn't a quoted string.
fn toml_str_value(line: &str, key: &str) -> Option<String> {
    let rest = line.strip_prefix(key)?.trim_start();
    let rest = rest.strip_prefix('=')?.trim();
    rest.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| rest.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .map(str::to_string)
}

pub fn fallback_models(agent_kind: &str) -> Vec<ConfigOptionValue> {
    match agent_kind {
        // The `claude` CLI has no model-listing command (unlike `opencode
        // models`), so there is nothing to probe. What it *does* expose is
        // `--model`, which accepts an alias for the latest model
        // ('opus'/'sonnet'/'haiku'/'fable') or a full model id. We store the
        // aliases here: they're passed straight through to `--model` and the
        // CLI resolves them to the current generation at runtime, so this list
        // never goes stale. Users wanting a pinned build can type a full id in
        // the custom-override field of the model picker.
        //
        // Vision-capability flags: opus / sonnet / haiku / fable all resolve
        // to current vision-capable Claude generations. `fable` now maps to
        // Claude Fable 5 (GA — Anthropic's most capable model, with
        // high-resolution vision like Opus 4.7+), so it is flagged true; the
        // earlier research-preview build had no vision and was flagged false.
        "claude-code" => vec![
            ConfigOptionValue {
                value: "opus".into(),
                name: "Claude Opus (latest)".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "sonnet".into(),
                name: "Claude Sonnet (latest)".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "haiku".into(),
                name: "Claude Haiku (latest)".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "fable".into(),
                name: "Claude Fable (latest)".into(),
                description: None,
                supports_images: true,
            },
        ],
        // `codex` has no `models` subcommand to probe (like claude-code), so
        // we list the current known model ids passed straight to `codex exec
        // --model`. Slugs must match codex's bundled model-metadata registry,
        // or the CLI emits a per-turn "Model metadata for `<slug>` not found.
        // Defaulting to fallback metadata" warning and mis-accounts tokens.
        // The GPT-5.3/5.2 Codex and GPT-5.4 model ids were retired from that
        // registry. These GPT-5.6 variants are the current metadata-backed
        // set. A user pointing Codex at a custom provider (e.g. a MiniMax
        // endpoint in ~/.codex/config.toml) types their model id in the
        // custom-override field instead. All GPT-5 Codex models are
        // vision-capable.
        "codex" => vec![
            ConfigOptionValue {
                value: "gpt-5.6-terra".into(),
                name: "GPT-5.6 Terra".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "gpt-5.6-sol".into(),
                name: "GPT-5.6 Sol".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "gpt-5.6-luna".into(),
                name: "GPT-5.6 Luna".into(),
                description: None,
                supports_images: true,
            },
        ],
        "opencode" | "hermes" => vec![
            ConfigOptionValue {
                value: "anthropic/claude-3-5-sonnet-20241022".into(),
                name: "Claude 3.5 Sonnet (Latest)".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "openai/gpt-4o".into(),
                name: "GPT-4o".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "google/gemini-2.5-flash".into(),
                name: "Gemini 2.5 Flash".into(),
                description: None,
                supports_images: true,
            },
            ConfigOptionValue {
                value: "deepseek/deepseek-coder-v2".into(),
                name: "DeepSeek Coder V2".into(),
                description: None,
                supports_images: false,
            },
        ],
        _ => vec![],
    }
}

#[cfg(test)]
#[path = "../../tests/application/agent_probe.rs"]
mod tests;
