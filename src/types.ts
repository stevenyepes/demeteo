// Post-pivot types. Legacy supervisor/thread types were removed as part of
// the R7 cleanup; see AGENT_INTEGRATION.md §1 for the surviving surface.

export type ConfigOptionValue = {
  value: string;
  name: string;
  description?: string;
};

export interface ConfigOption {
  id: string;
  name: string;
  description?: string;
  category?: string;
  type: string;
  currentValue: string;
  options: ConfigOptionValue[];
}

export interface Workflow {
  id: string;
  name: string;
  description: string;
  is_starter: boolean;
  created_at: number;
  updated_at: number;
  schedule?: WorkflowSchedule | null;
}

export type StepConfig = {
  id: string;
  kind: 'agent' | 'parallel' | 'gate' | string;
  title: string;
  agent_kind?: string | null;
  prompt_template?: string | null;
  artifact_mode: 'full' | 'summary_only' | 'none' | string;
  on_failure?: string | null;
  max_iterations?: number | null;
  verifier?: VerifierConfig | null;
};

export interface WorkflowWithSteps extends Workflow {
  steps: StepConfig[];
  version: number;
  version_id: string;
}

export interface StepExecution {
  id: string;
  feature_id: string;
  step_id: string;
  step_index: number;
  step_kind: string;
  status: 'pending' | 'running' | 'awaiting_gate' | 'completed' | 'failed' | 'skipped' | 'interrupted' | string;
  cost_usd?: number | null;
  tokens?: number | null;
  wall_clock_secs?: number | null;
  artifact_path?: string | null;
  artifact_paths: string[];
  error_message?: string | null;
  created_at: number;
  updated_at: number;
}

export interface GateDecision {
  id: string;
  step_execution_id: string;
  decision?: 'approve' | 'redirect' | 'cancel' | string | null;
  feedback?: string | null;
  created_at: number;
}

export interface Feature {
  id: string;
  project_id: string;
  workflow_id?: string;
  title: string;
  status: string;
  total_cost: number;
  tokens?: number | null;
  duration: string;
  created_at: number;
  agent_kind?: string | null;
  model?: string | null;
  /** URL of the published PR/MR, if any. Set by the `MrPublisher`. */
  mr_url?: string | null;
  /**
   * State of the PR/MR on the provider: `none | draft | open | merged | closed`.
   * `none` → no MR has been published. `open` is the typical "review pending"
   * state. The UI shows this as a badge on the feature detail.
   */
  mr_state?: string | null;
}

export type MrState = 'none' | 'draft' | 'open' | 'merged' | 'closed';

/** Return shape for `feature_sync` and `feature_resolve_sync_conflicts`. */
export type SyncOutcomeView =
  | {
      status: 'ok';
      merge_commit_sha: string;
      changed: boolean;
    }
  | {
      status: 'conflict';
      conflict_files: ConflictFile[];
      raw_error: string;
    }
  | {
      status: 'resolved';
      merge_commit_sha: string;
      revalidated_step_id: string | null;
    }
  | {
      status: 'resolution_failed';
      reason: string;
      conflict_files: ConflictFile[];
    };

export interface ConflictFile {
  path: string;
  /** "both-modified" | "added-by-them" | "added-by-us" | "deleted-by-them" | "deleted-by-us". */
  kind: string;
}

export interface Repository {
  id: string;
  repo_path: string;
}

export interface VerifierConfig {
  agent_kind?: string | null;
  instructions: string;
  harness_name?: string | null;
  verdict_key?: string;
}

export interface WorkflowSchedule {
  cron: string;
  title_template: string;
  project_id: string;
  next_run_at?: number | null;
}

export interface ProjectMemoryEntry {
  id: string;
  project_id: string;
  key: string;
  value: string;
  source: 'agent' | 'human';
  confidence: number;
  created_at: number;
  updated_at: number;
}

/**
 * Discriminated-union mirror of the Rust `AppError` enum.
 * Stable across releases — the `kind` field is the IPC contract;
 * do not rename variants without coordinating with the backend.
 */
export type AppErrorKind =
  | 'not_found'
  | 'validation'
  | 'conflict'
  | 'provider'
  | 'transport'
  | 'database'
  | 'agent'
  | 'internal';

export interface AppError {
  kind: AppErrorKind;
  message: string;
}
