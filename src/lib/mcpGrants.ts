import { invoke } from "@tauri-apps/api/core";

/** A Settings-visible active MCP grant. Mirrors the Rust `McpGrantSummary`
 *  exactly, including field casing — Tauri's default IPC serialization does
 *  not camelCase a Serialize payload, so this stays snake_case like
 *  `ProviderInstance`. `revoked` is always `false` in practice: the backend
 *  query already excludes revoked and expired rows before this shape exists. */
export interface McpGrantSummary {
  id: string;
  client_name: string;
  scopes: string[];
  issued_at: number;
  expires_at: number;
  revoked: boolean;
  /** The listener is bound elsewhere than this grant's audience, so its token
   *  is rejected until the client re-authorizes. */
  audience_mismatch: boolean;
}

/** Every active grant, revoked and expired rows already excluded. */
export async function listMcpGrants(): Promise<McpGrantSummary[]> {
  return invoke<McpGrantSummary[]>("list_mcp_grants");
}

/** Revoke a grant by id. Idempotent on the backend. */
export async function revokeMcpGrant(grantId: string): Promise<void> {
  return invoke<void>("revoke_mcp_grant", { grantId });
}

/** Resolve a pending MCP consent prompt, approved or denied. */
export async function decideMcpConsent(requestId: string, approve: boolean): Promise<void> {
  return invoke<void>("mcp_consent_decide", { requestId, approve });
}
