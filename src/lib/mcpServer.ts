import { invoke } from "@tauri-apps/api/core";

/** Mirrors the Rust `McpServerStatus` (`commands/mcp_server.rs`). `url` is
 *  the `/mcp` endpoint a client is configured with, not the bare origin, and
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

export async function installMcpSkill(destPath: string): Promise<void> {
  return invoke<void>("install_mcp_skill", { destPath });
}

/** Mirrors the Rust `McpConnectionTest` (`commands/mcp_server.rs`) — a
 *  failed probe is a meaningful answer to render, not a thrown error. */
export type McpConnectionTest =
  | { status: "reachable"; tool_count: number }
  | { status: "unreachable"; reason: string };

export async function testMcpConnection(): Promise<McpConnectionTest> {
  return invoke<McpConnectionTest>("test_mcp_connection");
}
