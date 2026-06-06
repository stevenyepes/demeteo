export interface Machine {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_type: string;
  key_path?: string;
  agents?: string; // JSON array string
  auto_approved_rules?: string; // JSON array string
}

export interface FrontendMachine extends Machine {
  type: 'local' | 'server';
  status: 'connected' | 'offline';
  user: string;
}

export interface ThreadSession {
  id: string;
  machine_id: string;
  title: string;
  mode: string; // 'worktree' | 'adhoc'
  branch?: string;
  repo_path?: string;
  sandbox_path?: string;
  status: string; // 'idle' | 'running' | 'pending_approval'
}

export interface FileReference {
  name: string;
  lines: number;
  type: string;
  isNew?: boolean;
}

export interface StreamEvent {
  id: string;
  type: "directive" | "info" | "auto_approve" | "intercept";
  message: string;
  timestamp: string;
  payload?: {
    path: string;
    additions: number;
    code: string;
  };
}
