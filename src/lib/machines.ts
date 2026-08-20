import { invoke } from "@tauri-apps/api/core";
import type { Machine } from "../types";

export async function listMachines(): Promise<Machine[]> {
  return invoke<Machine[]>("get_machines");
}

export async function addMachine(machine: Machine): Promise<void> {
  return invoke<void>("add_machine", { machine });
}

export async function updateMachine(machine: Machine): Promise<void> {
  return invoke<void>("update_machine", { machine });
}

export async function deleteMachine(id: string): Promise<void> {
  return invoke<void>("delete_machine", { id });
}

/** Store the machine's SSH secret (key passphrase or password) in the OS
 *  keyring. Never persisted anywhere else — see AGENTS.md §2. */
export async function setMachineSecret(machineId: string, secret: string): Promise<void> {
  return invoke<void>("set_machine_secret", { machineId, secret });
}

export async function deleteMachineSecret(machineId: string): Promise<void> {
  return invoke<void>("delete_machine_secret", { machineId });
}

/** Open a connection to the machine and close it again. Resolves on
 *  success; the rejection carries the transport's own diagnosis. */
export async function testMachineConnection(machineId: string): Promise<void> {
  return invoke<void>("test_machine_connection", { machineId });
}

/**
 * One entry of the JSON array stored in `Machine.agents`. The column is a
 * string, not a relation, so every field is optional here: a row written by
 * an older build (or hand-edited) may be missing either one, and the pickers
 * that read it must degrade rather than throw.
 */
export interface MachineAgentRecord {
  kind?: string;
  enabled?: boolean;
}

/**
 * Decode `Machine.agents`. Malformed JSON, a non-array document and a
 * non-object entry all resolve to something harmless rather than throwing,
 * because this runs while rendering a machine card.
 */
export function parseMachineAgents(agentsJson: string | null | undefined): MachineAgentRecord[] {
  if (!agentsJson) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(agentsJson);
  } catch {
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  return parsed.map((entry) =>
    typeof entry === "object" && entry !== null ? (entry as MachineAgentRecord) : {},
  );
}

/** An agent kind on one machine, with the availability probe's verdict. The
 *  settings tab's half of the Rust `AgentConfigView` (`state.rs`); the row's
 *  containment answer is read off the same command by `AgentAvailability`,
 *  which is the run surface's own narrower view of it. */
export interface AgentConfigView {
  kind: string;
  enabled: boolean;
  available: boolean;
  install_command: string;
  display_label: string;
}

/**
 * The agents configured on a machine. `refresh` re-runs the availability
 * probe per agent (an SSH round-trip each) and updates the backend cache;
 * everything but the settings tab's explicit "Re-check" passes `false`.
 */
export async function getAgentConfigs(
  machineId: string,
  refresh = false,
): Promise<AgentConfigView[]> {
  return invoke<AgentConfigView[]>("get_agent_configs", { machineId, refresh });
}

/** The enabled/disabled selection only — availability is probed, never sent. */
export async function setAgentConfigs(
  machineId: string,
  agents: Array<{ kind: string; enabled: boolean }>,
): Promise<void> {
  return invoke<void>("set_agent_configs", { machineId, agents });
}
