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
  duration: string;
  created_at: number;
}

export interface Repository {
  id: string;
  repo_path: string;
}
