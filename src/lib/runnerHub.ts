import { invoke } from "@tauri-apps/api/core";
import type {
  HubConnection,
  HubLocalPolicy,
  HubMachine,
  HubRequestLogEntry,
  HubStatus,
} from "./hub";

/** Mirrors the Rust `DeviceTicket` (`domain/hub_pairing/device.rs`): the code
 *  to type on the Hub, where, and the runner's fingerprint to compare with
 *  the one the Hub shows. `expires_at` is Unix seconds by the runner's clock. */
export interface DeviceTicket {
  user_code: string;
  verification_uri: string;
  verification_uri_complete: string;
  fingerprint: string;
  expires_at: number;
}

/** Mirrors the Rust `PairStateView` (`domain/hub_pairing/runner_view.rs`).
 *  A pairing that completed reads `idle`; `remoteHubStatus` says paired. */
export type RunnerPairState =
  | { state: "idle" }
  | { state: "pending"; ticket: DeviceTicket }
  | { state: "failed"; error: string };

/** Mirrors the Rust `HubStatusView`. `store_unavailable` says why the runner's
 *  sealed store cannot hold Hub credentials, with the command that fixes it. */
export interface RunnerHubStatus {
  status: HubStatus;
  store_unavailable: string | null;
  connection: HubConnection;
  machine: HubMachine;
}

/** Every call reaches the runner on `machineId` over SSH, and rejects with
 *  the runner's own message unchanged. */
export async function remoteHubPairBegin(
  machineId: string,
  hubUrl: string,
): Promise<DeviceTicket> {
  return invoke<DeviceTicket>("remote_hub_pair_begin", { machineId, hubUrl });
}

export async function remoteHubPairState(machineId: string): Promise<RunnerPairState> {
  return invoke<RunnerPairState>("remote_hub_pair_state", { machineId });
}

export async function remoteHubPairCancel(machineId: string): Promise<RunnerPairState> {
  return invoke<RunnerPairState>("remote_hub_pair_cancel", { machineId });
}

/** `null` for a runner from before Hub pairing. */
export async function remoteHubStatus(machineId: string): Promise<RunnerHubStatus | null> {
  return invoke<RunnerHubStatus | null>("remote_hub_status", { machineId });
}

export async function remoteHubDisconnect(machineId: string): Promise<HubStatus> {
  return invoke<HubStatus>("remote_hub_disconnect", { machineId });
}

export async function remoteHubPolicyGet(machineId: string): Promise<HubLocalPolicy> {
  return invoke<HubLocalPolicy>("remote_hub_policy_get", { machineId });
}

/** Resolves to the policy the runner stored, which is what is in force there. */
export async function remoteHubPolicySet(
  machineId: string,
  policy: HubLocalPolicy,
): Promise<HubLocalPolicy> {
  return invoke<HubLocalPolicy>("remote_hub_policy_set", { machineId, policy });
}

/** Newest first; `null` for a runner from before its activity log. */
export async function remoteHubRequestLog(
  machineId: string,
  limit: number,
): Promise<HubRequestLogEntry[] | null> {
  return invoke<HubRequestLogEntry[] | null>("remote_hub_request_log", { machineId, limit });
}
