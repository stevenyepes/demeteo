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

/** Mirrors the Rust `ConnectionState` (`domain/hub_socket/connection.rs`). */
export type HubConnectionState =
  | "not_running"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stopped";

/** Mirrors the Rust `SyncState`. Anything but `connected` is `offline`. */
export type HubSyncState = "in_sync" | "catching_up" | "not_reporting" | "offline";

/** Mirrors the Rust `StopReason` (`domain/hub_socket/verdict.rs`). */
export type HubStopReason =
  | "displaced"
  | "protocol_violation"
  | "unauthorized"
  | "refused"
  | "needs_repair"
  | "refresh_uncertain"
  | "clock_skew"
  | "stopped";

/** Mirrors the Rust `HubConnection`. `latency_ms` is null unless connected;
 *  `reported_through` is the last offset the Hub acked since the app started,
 *  not a stored cursor. */
export interface HubConnection {
  state: HubConnectionState;
  stop_reason: HubStopReason | null;
  latency_ms: number | null;
  reported_through: number | null;
  sync: HubSyncState;
  last_heard_at_ms: number | null;
}

/** From the desktop's memory only; the Hub is not contacted. */
export async function hubConnection(): Promise<HubConnection> {
  return invoke<HubConnection>("hub_connection");
}

/** Mirrors the Rust `HubMachine` (`domain/hub_pairing/mod.rs`): the labels
 *  pairing sends the Hub's consent page, recomputed rather than stored. */
export interface HubMachine {
  instance_name: string;
  os: string;
  arch: string;
  app_version: string;
}

export async function hubMachine(): Promise<HubMachine> {
  return invoke<HubMachine>("hub_machine");
}

/** Mirrors the Rust `PasskeyOrigin` (`domain/hub_request/passkeys.rs`):
 *  `pinned` came with the pairing, `endorsed` was added later. */
export type HubPasskeyOrigin = "pinned" | "endorsed";

/** Mirrors the Rust `HubTrustedPasskey`; it carries no public key. */
export interface HubTrustedPasskey {
  credential_id: string;
  sign_count: number;
  pinned_at: number;
  origin: HubPasskeyOrigin;
}

/** In pairing order; empty when unpaired, which is not an error. */
export async function hubTrustedPasskeys(): Promise<HubTrustedPasskey[]> {
  return invoke<HubTrustedPasskey[]>("hub_trusted_passkeys");
}

/** Forgets the pairing locally and is idempotent; the Hub is not contacted. */
export async function hubDisconnect(): Promise<HubStatus> {
  return invoke<HubStatus>("hub_disconnect");
}

/** Mirrors the Rust `HubLocalPolicy` (`domain/hub_request/mod.rs`). Set only
 *  from this desktop; no Hub request can change it. An unset ceiling refuses
 *  every Hub start. */
export interface HubLocalPolicy {
  read_enabled: boolean;
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

/** Mirrors the Rust `HubRequestLogRow`: what was asked and answered, never
 *  what the request carried, save the credential ids an applied passkey
 *  endorsement or revocation names in `reason`. `id` is the log row's own
 *  key; `request_id` may repeat, since a redelivery and a deferred request
 *  handled later are each logged again. */
export interface HubRequestLogEntry {
  id: number;
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
