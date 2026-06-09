export interface Machine {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: string;
  key_path?: string;
  agents?: string; // JSON array string; legacy bare strings auto-migrated to AgentConfig
  auto_approved_rules?: string; // JSON array string (legacy)
}

export interface FrontendMachine extends Machine {
  type: 'local' | 'server';
  status: 'connected' | 'offline';
  user: string;
}

export interface AgentConfig {
  kind: string;
  enabled: boolean;
}

export interface AgentConfigView {
  kind: string;
  enabled: boolean;
  available: boolean;
  install_command: string;
}

export type AgentKind = "opencode" | "hermes" | "noop";

export type ThreadStatus =
  | "idle"
  | "running"
  | "pending_approval"
  | "spawning"
  | "installing"
  | "error";

export interface ThreadSession {
  id: string;
  machine_id: string;
  title: string;
  mode: string; // 'worktree' | 'adhoc'
  branch?: string;
  repo_path?: string;
  sandbox_path?: string;
  status: ThreadStatus | string; // backend enforces; UI accepts the union plus legacy strings
  agent_kind?: AgentKind | string | null;
}

export interface WorkingMemoryEntry {
  file_path: string;
  line_count?: number | null;
  size_bytes?: number | null;
  modified_at?: number | null;
  first_read_at: number;
  last_read_at: number;
}

export interface FileReference {
  name: string;
  lines: number;
  type: string;
  isNew?: boolean;
}

export type ActionKind = "read" | "edit" | "write" | "run_bash";

export interface InterceptPayload {
  intercept_id: string;
  thread_id: string;
  machine_id: string;
  action: ActionKind;
  target: string;
  preview?: string;
  created_at: string;
  tool_call_id?: string | null;
}

export type ExecutionResult =
  | { kind: "bash"; output: string }
  | { kind: "file_changed"; path: string; lines_added: number; lines_removed: number }
  | { kind: "file_read"; path: string; content_preview: string };

export type CommandOutcome =
  | { kind: "executed"; output: ExecutionResult }
  | { kind: "intercepted"; intercept_id: string; payload: InterceptPayload };

export type AgentAction =
  | { Read: { path: string } }
  | { Edit: { path: string; content: string } }
  | { Write: { path: string; content: string } }
  | { RunBash: { cmd: string } };

export function actionToString(a: AgentAction): string {
  if ("Read" in a) return `read ${a.Read.path}`;
  if ("Edit" in a) return `edit ${a.Edit.path}`;
  if ("Write" in a) return `write ${a.Write.path}`;
  return `run ${a.RunBash.cmd}`;
}

// Agent streaming events (AGENT_INTEGRATION §3.2). Phase 7a defines the shape;
// Phase 7c wires the per-turn Channel.
export type AgentEvent =
  | { kind: "text"; delta: string }
  | {
      kind: "tool_call";
      tool_call_id: string;
      intercept_id: string;
      action: ActionKind;
      target: string;
      preview?: string;
    }
  | {
      kind: "tool_call_update";
      tool_call_id: string;
      status:
        | { status: "pending" }
        | { status: "in_progress"; message?: string }
        | { status: "completed" }
        | { status: "failed"; reason: string };
      preview?: string;
    }
  | { kind: "plan"; entries: { step: string; status: string }[] }
  | {
      kind: "usage";
      input_tokens: number;
      output_tokens: number;
      cost_usd?: number | null;
    }
  | {
      kind: "error";
      code: string;
      message: string;
      recoverable: boolean;
    }
  | {
      kind: "turn_complete";
      stop_reason: "end_of_turn" | "cancelled" | "max_tokens" | "error";
    };

export interface ThreadStatusChangedEvent {
  thread_id: string;
  status: ThreadStatus | string;
  reason?: string;
}

export interface StreamEvent {
  id: string;
  type: "directive" | "info" | "auto_approve" | "intercept" | "agent_error" | "text";
  message: string;
  timestamp: string;
  payload?: {
    intercept_id?: string;
    action?: ActionKind;
    path?: string;
    additions?: number;
    code?: string;
    created_at?: string;
    tool_call_id?: string | null;
    feedback?: string;
  };
}
