import { invoke } from "@tauri-apps/api/core";

/** Mirrors the Rust `HubPairingState` (`domain/hub_pairing/mod.rs`). Not
 *  liveness: `paired` means only that the pins and both keyring entries are
 *  present locally; `incomplete` is any partial set, which includes pins
 *  behind an unavailable keyring. */
export type HubPairingState = "unpaired" | "paired" | "incomplete";

/** Mirrors the Rust `HubStatus` exactly, snake_case included — Tauri's IPC
 *  does not camelCase a Serialize payload. `hub_url` is the normalized origin,
 *  and `fingerprint` is the Hub's own `SHA256:xxxx …` spelling so the user can
 *  compare the two by eye. */
export interface HubStatus {
  state: HubPairingState;
  hub_url: string | null;
  instance_id: string | null;
  fingerprint: string | null;
  rp_id: string | null;
  scopes: string[];
  paired_at: number | null;
  keyring_backend: string;
  keyring_available: boolean;
}

/** Opens the Hub's consent page in the browser and resolves once the loopback
 *  callback completes. Rejects with a plain message string. */
export async function hubPair(hubUrl: string): Promise<HubStatus> {
  return invoke<HubStatus>("hub_pair", { hubUrl });
}

export async function hubStatus(): Promise<HubStatus> {
  return invoke<HubStatus>("hub_status");
}

/** Forgets the pairing locally and is idempotent; the Hub is not contacted. */
export async function hubDisconnect(): Promise<HubStatus> {
  return invoke<HubStatus>("hub_disconnect");
}

/** Mirrors the Rust `HubLocalPolicy` (`domain/hub_request/mod.rs`). Set only
 *  from this desktop; no Hub request can change it. An unset ceiling refuses
 *  every Hub start. */
export interface HubLocalPolicy {
  spend_enabled: boolean;
  gates_enabled: boolean;
  run_ceiling_cents: number | null;
  daily_ceiling_cents: number | null;
}

/** Mirrors the Rust `LogKind`; `unknown` is a body that did not parse. */
export type HubRequestKind =
  | "start_feature"
  | "cancel_feature"
  | "set_step_assignment"
  | "gate_decision"
  | "passkey_endorsement"
  | "passkey_revocation"
  | "unknown";

export type HubScope = "read" | "spend" | "gates";

/** Mirrors the Rust `LogOutcome`. `dropped` and `deferred` were not answered. */
export type HubRequestOutcome = "accepted" | "refused" | "dropped" | "deferred";

/** Mirrors the Rust `HubRequestLogEntry`: what was asked and answered, never
 *  what the request carried. */
export interface HubRequestLogEntry {
  request_id: string;
  kind: HubRequestKind;
  scope: HubScope | null;
  outcome: HubRequestOutcome;
  reason: string | null;
  feature_id: string | null;
  received_at_ms: number;
}

export async function hubPolicyGet(): Promise<HubLocalPolicy> {
  return invoke<HubLocalPolicy>("hub_policy_get");
}

export async function hubPolicySet(policy: HubLocalPolicy): Promise<void> {
  return invoke<void>("hub_policy_set", { policy });
}

/** Newest first. */
export async function hubRequestLog(limit: number): Promise<HubRequestLogEntry[]> {
  return invoke<HubRequestLogEntry[]>("hub_request_log", { limit });
}
