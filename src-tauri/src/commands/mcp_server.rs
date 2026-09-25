use crate::ports::db::AppSettingsRepository;
use crate::state::AppContext;
use tauri::State;

/// The skill's own markdown, embedded at compile time so "Install skill" has
/// no runtime dependency on the docs tree being present alongside the built
/// app.
const MCP_SKILL_MARKDOWN: &str = include_str!("../../../docs/mcp-skill/SKILL.md");

/// Mirrors `demeteo_core::adapters::mcp::MCP_SERVER_ENABLED_KEY` — private
/// to that crate and not reachable across the crate boundary, the same
/// reason `tests/adapters/mcp/gating.rs` mirrors the key constant directly
/// rather than importing it.
const MCP_SERVER_ENABLED_KEY: &str = "mcp_server_enabled";

/// The Preferences-screen shape: whether the MCP listener is enabled, and
/// the endpoint URL (`demeteo_core::adapters::mcp::endpoint_url`) to show
/// once it is — what a client is configured with and "Test connection"
/// probes, never the bare canonical origin.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct McpServerStatus {
    pub enabled: bool,
    pub url: Option<String>,
}

/// Same read-with-default idiom as `resolve_enabled` in
/// `demeteo_core::adapters::mcp` (private there), without its
/// persist-the-default side effect — a status read should not write.
fn resolve_enabled(store: &dyn AppSettingsRepository) -> bool {
    matches!(store.app_setting_get(MCP_SERVER_ENABLED_KEY), Ok(Some(raw)) if raw.trim() == "true")
}

/// Command core for [`get_mcp_server_status`], factored out to take the
/// settings store directly per `commands/app_session.rs`'s split
/// (`State<'_, AppContext>` cannot be built in a test — see
/// `tests/infrastructure/oauth.rs`). `enabled` reflects persisted intent;
/// `url` reflects the live `demeteo_core::adapters::mcp::endpoint_url`
/// state, so a failed or not-yet-attempted bind reports `url: None` even
/// when `enabled` is `true`.
pub fn read_mcp_server_status(store: &dyn AppSettingsRepository) -> McpServerStatus {
    McpServerStatus {
        enabled: resolve_enabled(store),
        url: crate::adapters::mcp::endpoint_url(),
    }
}

/// Command core for [`set_mcp_server_enabled`]: the live-toggle seam,
/// delegating persistence and the actual bind/unbind to
/// `demeteo_core::adapters::mcp::set_enabled`. Its own effect on this same
/// process's listener is proven at that level
/// (`tests/adapters/mcp/gating.rs`), not re-proven here — this ticket's test
/// covers the settings round-trip [`read_mcp_server_status`] decodes.
pub fn write_mcp_server_enabled(ctx: AppContext, runtime: &tokio::runtime::Handle, enabled: bool) {
    crate::adapters::mcp::set_enabled(ctx, runtime, enabled);
}

#[tauri::command]
pub fn get_mcp_server_status(ctx: State<'_, AppContext>) -> Result<McpServerStatus, String> {
    Ok(read_mcp_server_status(ctx.app_settings.as_ref()))
}

#[tauri::command]
pub fn set_mcp_server_enabled(ctx: State<'_, AppContext>, enabled: bool) -> Result<(), String> {
    write_mcp_server_enabled(
        ctx.inner().clone(),
        &tauri::async_runtime::handle().inner().clone(),
        enabled,
    );
    Ok(())
}

/// Command core for [`install_mcp_skill`]: writes the bundled skill markdown
/// to `dest_path`. The webview cannot write a file itself, so the frontend
/// resolves `dest_path` via the OS save dialog and this only writes to
/// wherever the user pointed it — same division as
/// `ask::ask_export_canvas_to_file`.
pub fn write_mcp_skill(dest_path: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(dest_path, MCP_SKILL_MARKDOWN)
}

#[tauri::command]
pub fn install_mcp_skill(dest_path: String) -> Result<(), String> {
    write_mcp_skill(std::path::Path::new(&dest_path)).map_err(|e| e.to_string())
}

/// Preferences-screen "Test connection" result. A failed probe is not a
/// command error — it is a meaningful answer the UI renders — so it lives in
/// this `Ok` variant rather than the command's `Err` string, the same split
/// `mcp_handler.rs`'s own `tool_success`/`tool_failure` draws between a
/// protocol failure and an operation that ran and reported failure.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum McpConnectionTest {
    Reachable,
    Unreachable { reason: String },
}

/// Command core for [`test_mcp_connection`]: a same-machine `tools/list`
/// probe, sent the way a client's first request arrives — no bearer token,
/// and no `Origin` header, because `demeteo_core::adapters::mcp::origin`'s
/// DNS-rebinding guard refuses even this app's own webview (its `Origin`
/// never equals the listener's canonical URI), which is why this runs in
/// Rust and not as a frontend `fetch()`. Healthy is therefore the `401`
/// whose `resource_metadata` challenge is what starts a client's sign-in
/// (`docs/MCP_INTEGRATION.md` §5); anything else means a client would stall.
pub async fn probe_mcp_connection(client: &reqwest::Client, url: &str) -> McpConnectionTest {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {},
    });

    let response = match client.post(url).json(&body).send().await {
        Ok(response) => response,
        Err(e) => {
            return McpConnectionTest::Unreachable {
                reason: e.to_string(),
            }
        }
    };
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return McpConnectionTest::Unreachable {
            reason: format!("server responded with {}", response.status()),
        };
    }
    let challenges_for_sign_in = response
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("Bearer ") && value.contains("resource_metadata="));
    if challenges_for_sign_in {
        McpConnectionTest::Reachable
    } else {
        McpConnectionTest::Unreachable {
            reason: "server responded with 401 but no sign-in challenge".to_string(),
        }
    }
}

#[tauri::command]
pub async fn test_mcp_connection(ctx: State<'_, AppContext>) -> Result<McpConnectionTest, String> {
    let status = read_mcp_server_status(ctx.app_settings.as_ref());
    let Some(url) = status.url else {
        return Ok(McpConnectionTest::Unreachable {
            reason: "MCP server is not listening".to_string(),
        });
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(probe_mcp_connection(&client, &url).await)
}

#[cfg(test)]
#[path = "../../tests/infrastructure/mcp_server.rs"]
mod tests;
