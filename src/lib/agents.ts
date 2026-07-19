// Single source of truth for the coding-agent CLIs the terminal panel
// knows about. The `+ New` menu launches them, and both the frontend
// (badges) and the Rust foreground detector key off the same `kind`
// identifiers — keep this in sync with `agent_kind_for_binary` in
// `src-tauri/src/terminal.rs`.

export interface AgentMeta {
  /** Stable kind id shared with the backend (e.g. `"claude-code"`). */
  kind: string;
  /** Executable to run in the shell to launch the agent. */
  binary: string;
  /** Human label shown in menus and badges. */
  label: string;
}

export const AGENTS: Record<string, AgentMeta> = {
  'claude-code': { kind: 'claude-code', binary: 'claude', label: 'Claude' },
  opencode: { kind: 'opencode', binary: 'opencode', label: 'OpenCode' },
  hermes: { kind: 'hermes', binary: 'hermes', label: 'Hermes' },
  codex: { kind: 'codex', binary: 'codex', label: 'Codex' },
};

/** The agents surfaced when a machine reports no explicit agent config. */
export function defaultAgentKinds(): string[] {
  return ['claude-code', 'opencode'];
}

/** Resolve a display label for an agent kind, falling back to the raw kind
 *  (title-cased) for an agent the frontend doesn't have metadata for. */
export function agentLabel(kind: string | null | undefined): string | null {
  if (!kind) return null;
  const meta = AGENTS[kind];
  if (meta) return meta.label;
  // Unknown kind (e.g. a future backend addition) — present it readably
  // rather than dropping the badge entirely.
  return kind.charAt(0).toUpperCase() + kind.slice(1);
}
