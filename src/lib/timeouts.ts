import { invoke } from "@tauri-apps/api/core";
import type { AgentTimeouts } from "../types";

/** Read the persisted global agent timeouts (defaults when unset). */
export async function getAgentTimeouts(): Promise<AgentTimeouts> {
  return invoke<AgentTimeouts>("get_agent_timeouts");
}

/** Persist a new agent timeouts configuration. The backend validates the
 * input (monotonicity: `normal >= fast`, `wall >= normal`; bounds on each
 * field) and rejects out-of-range values with a `Validation` error. */
export async function setAgentTimeouts(config: AgentTimeouts): Promise<void> {
  return invoke<void>("set_agent_timeouts", { config });
}