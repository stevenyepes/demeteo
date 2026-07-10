import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/**
 * A registered, user-selectable coding agent and the capabilities Demeteo
 * asks of it. Mirrors the Rust `AgentCatalogEntry` (`state.rs`) returned by
 * the `list_agents` command — the single source of truth for "which agents
 * exist", replacing the hardcoded `AGENT_KINDS` arrays each component used to
 * keep in sync by hand.
 */
export interface AgentCatalogEntry {
  kind: string;
  display_label: string;
  lists_models: boolean;
  default_model: string | null;
  install_command: string;
}

let cache: AgentCatalogEntry[] | null = null;
let inflight: Promise<AgentCatalogEntry[]> | null = null;

/** Fetch the agent catalog once and memoize it for the app session. */
export function loadAgentCatalog(): Promise<AgentCatalogEntry[]> {
  if (cache) return Promise.resolve(cache);
  if (!inflight) {
    inflight = invoke<AgentCatalogEntry[]>('list_agents')
      .then((list) => {
        cache = list;
        return list;
      })
      .finally(() => {
        inflight = null;
      });
  }
  return inflight;
}

/** Human-facing label for a kind, falling back to the slug if unknown. */
export function agentLabel(catalog: AgentCatalogEntry[], kind: string): string {
  return catalog.find((a) => a.kind === kind)?.display_label ?? kind;
}

/**
 * React hook exposing the memoized agent catalog. Returns an empty list until
 * the first fetch resolves; callers that need the kinds synchronously read
 * `agents.map((a) => a.kind)`.
 */
export function useAgentCatalog(): { agents: AgentCatalogEntry[]; loading: boolean } {
  const [agents, setAgents] = useState<AgentCatalogEntry[]>(cache ?? []);
  const [loading, setLoading] = useState(cache === null);

  useEffect(() => {
    let alive = true;
    loadAgentCatalog()
      .then((list) => {
        if (alive) {
          setAgents(list);
          setLoading(false);
        }
      })
      .catch(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, []);

  return { agents, loading };
}
