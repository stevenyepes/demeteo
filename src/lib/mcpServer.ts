import { invoke } from "@tauri-apps/api/core";

/** Mirrors the Rust `McpServerStatus` (`commands/mcp_server.rs`). `url` is
 *  `null` whenever the listener is disabled or not yet bound. */
export interface McpServerStatus {
  enabled: boolean;
  url: string | null;
}

export async function getMcpServerStatus(): Promise<McpServerStatus> {
  return invoke<McpServerStatus>("get_mcp_server_status");
}

export async function setMcpServerEnabled(enabled: boolean): Promise<void> {
  return invoke<void>("set_mcp_server_enabled", { enabled });
}
