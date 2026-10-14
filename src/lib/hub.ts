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
