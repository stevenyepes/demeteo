import { invoke } from "@tauri-apps/api/core";
import type { ConfigOptionValue } from "../types";

type ModelList = ConfigOptionValue[];

const cache = new Map<string, Promise<ModelList>>();
const resolved = new Map<string, ModelList>();

function key(machineId: string, agentKind: string): string {
  return `${machineId}::${agentKind}`;
}

/**
 * Fetch the available model list for `(machineId, agentKind)`.
 *
 * **Dedupe + cache.** The Tauri command `get_agent_models` spawns a
 * short-lived `opencode acp` over SSH to introspect the agent's
 * `session/new` response (see `commands/agent_config_probe.rs`). Calling
 * it many times in a row — e.g. on every React re-render or model-picker
 * open — opens a fresh SSH channel each time and overloads the remote
 * server. This wrapper:
 *
 *   - Returns the cached result for a key we already resolved.
 *   - Shares a single in-flight Promise for concurrent callers of the
 *     same key (so a useEffect double-fire in dev/StrictMode doesn't
 *     double-spawn the agent).
 *   - Caches the resolved list permanently for the app session — the
 *     model list rarely changes mid-session, and re-probing costs a
 *     remote `opencode acp` spawn each time.
 *
 * The cache is module-level so it survives component remounts. The
 * Tauri command itself is also fixed: it now calls `session.kill()`
 * before dropping the registry entry, so even an uncached call no
 * longer leaves an orphan process on the server.
 */
export async function getAgentModels(
  machineId: string,
  agentKind: string,
): Promise<ModelList> {
  const k = key(machineId, agentKind);

  const hit = resolved.get(k);
  if (hit) return hit;

  const inflight = cache.get(k);
  if (inflight) return inflight;

  const promise = (async () => {
    try {
      const models = await invoke<ModelList>("get_agent_models", {
        machineId,
        agentKind,
      });
      const list = models || [];
      resolved.set(k, list);
      return list;
    } finally {
      cache.delete(k);
    }
  })();

  cache.set(k, promise);
  return promise;
}

/** Test-only escape hatch. */
export function _resetAgentModelsCache() {
  cache.clear();
  resolved.clear();
}
