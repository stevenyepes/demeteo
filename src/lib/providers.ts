import { invoke } from "@tauri-apps/api/core";

/** A connected Git hosting account. Mirrors the Rust `ProviderInstance`;
 *  the PAT is not part of it — the token lives in the OS keyring and the
 *  frontend never sees it again after the connect call. */
export interface ProviderInstance {
  id: string;
  kind: string;
  host: string;
  username: string;
  avatar_url: string;
  created_at: number;
}

/** Every connected instance. The PAT is absent from each row — it stays in
 *  the keyring, so a caller wanting to act as the user must go through a
 *  command, never through these fields. */
export async function listProviderInstances(): Promise<ProviderInstance[]> {
  return invoke<ProviderInstance[]>("list_provider_instances");
}

/** Validate the PAT against the host and persist the instance. Rejects with
 *  the provider's own 401/403 verbatim so the form can be corrected. */
export async function connectProviderInstance(
  providerType: string,
  host: string,
  pat: string,
): Promise<ProviderInstance> {
  return invoke<ProviderInstance>("connect_provider_instance", { providerType, host, pat });
}

/** Delete the instance and its keyring credential. */
export async function deleteProviderInstance(providerId: string): Promise<void> {
  return invoke<void>("delete_provider_instance", { providerId });
}

/** Every repository path the instance's PAT can see. */
export async function fetchProviderRepos(providerId: string): Promise<string[]> {
  return invoke<string[]>("fetch_provider_repos", { providerId });
}
