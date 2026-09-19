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
/// the URL to show once it is.
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
/// `url` reflects the live `demeteo_core::adapters::mcp::canonical_uri`
/// state, so a failed or not-yet-attempted bind reports `url: None` even
/// when `enabled` is `true`.
pub fn read_mcp_server_status(store: &dyn AppSettingsRepository) -> McpServerStatus {
    McpServerStatus {
        enabled: resolve_enabled(store),
        url: crate::adapters::mcp::canonical_uri(),
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

#[cfg(test)]
#[path = "../../tests/infrastructure/mcp_server.rs"]
mod tests;
