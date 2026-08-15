import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { supportedEffortsFor, type EffortLevel } from './effortLevels';

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
  /**
   * The effort levels this agent accepts per invocation, straight from its
   * declared Rust `AgentCapabilities`. Empty (hermes) means the agent has no
   * per-invocation effort control at all and the picker must not offer one.
   * Older backends omit the field entirely, hence the optional marker.
   */
  effort_levels?: EffortLevel[];
  /**
   * What Demeteo's own spawn flags do to the setup this harness would
   * otherwise load on the user's machine — the Rust `PersonalizationSupport`,
   * kebab-serialized. Absent from a backend that predates the field.
   */
  personalization?: PersonalizationSupport;
}

/** Mirrors the Rust `PersonalizationSupport` wire form. */
export type PersonalizationSupport = 'loaded' | 'suppressed' | 'native';

/**
 * What a run on `kind` does to the user's own setup — the *effective* answer,
 * which the harness alone cannot give.
 *
 * The catalog declares what Demeteo's flags do to a step that does not keep the
 * harness's personalization. A step that keeps it (`StepConfig.uses_agent_skills`
 * in Rust) has none of those flags emitted on any harness, so `suppressed`
 * becomes `loaded`. `native` is untouched: that adapter reads no such switch, so
 * there is nothing for a step to keep and claiming otherwise would invent a
 * guarantee.
 *
 * `null` is "nobody has said" — no kind chosen, the catalog still loading, or a
 * backend that predates the field — and is deliberately not `'native'`. Every
 * value here is a claim about what a run is about to do to work the user did
 * themselves, and the honest answer to an unloaded catalog is silence; the
 * static fallback `effortLevelsFor` leans on has no counterpart for that
 * reason.
 *
 * Every surface asks through here, so the answer has one place to be resolved
 * rather than one per call site — and a surface rendering the declared value
 * beside a run that dropped those flags is the lie this resolution exists to
 * stop.
 */
export function personalizationFor(
  catalog: AgentCatalogEntry[],
  kind: string | null | undefined,
  stepKeepsPersonalization: boolean,
): PersonalizationSupport | null {
  if (!kind) return null;
  const declared = catalog.find((a) => a.kind === kind)?.personalization;
  if (declared === undefined || !KNOWN_SUPPORT.includes(declared)) return null;
  if (declared === 'suppressed' && stepKeepsPersonalization) return 'loaded';
  return declared;
}

/**
 * Rejecting a spelling this frontend does not know is what makes a rename on
 * either side of the wire silent rather than fatal: the note stops rendering,
 * which is also what "nobody has declared an answer" looks like on that
 * surface. Without the check, an unrecognized value reaches the note's lookup
 * tables as a missing key and takes the whole surface down mid-render.
 */
const KNOWN_SUPPORT: readonly PersonalizationSupport[] = ['loaded', 'suppressed', 'native'];

/**
 * The effort levels `kind` accepts, per the backend catalog. Falls back to the
 * hand-mirrored static table when the catalog hasn't loaded yet (or predates
 * `effort_levels`), so the picker is never wrongly greyed out mid-fetch.
 */
export function effortLevelsFor(
  catalog: AgentCatalogEntry[],
  kind: string,
): readonly EffortLevel[] {
  const entry = catalog.find((a) => a.kind === kind);
  return entry?.effort_levels ?? supportedEffortsFor(kind);
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
