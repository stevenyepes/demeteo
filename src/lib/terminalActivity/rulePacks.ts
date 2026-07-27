// Terminal-activity Phase 3 — rule packs (T3.1).
//
// On-screen "needs a decision" recognition for agents that do NOT self-report
// via injected hooks (Codex, OpenCode, hand-started). The recognizer
// (`recognizer.ts`) matches the *rendered* bottom rows of a terminal against a
// per-agent rule pack; a match promotes `awaiting_input → awaiting_approval`
// (strict approval-only — see docs/TERMINAL_ACTIVITY.md §Phase 3).
//
// The packs are DATA, not code: adding an agent (or re-tuning a drifted prompt
// UI) is authoring an entry in `DEFAULT_RULE_PACKS`, not touching the engine.
// They are bundled today; the loader is written so a remotely-fetched pack
// (plan §7.3) can be swapped in by re-calling `compileRulePacks` — agent UIs
// drift, so recognition config is expected to be centrally patchable.

/**
 * One approval-recognition rule. A rule MATCHES the scanned screen text when
 * **every** `all` pattern is present, **at least one** `any` pattern is present
 * (when `any` is non-empty), and **no** `none` pattern is present. At least one
 * of `all`/`any` must be non-empty — a rule that constrains nothing would match
 * every frame and is rejected at compile time.
 *
 * Patterns are JavaScript regular-expression source strings, matched
 * case-insensitively against the joined bottom rows of the rendered grid (ANSI
 * already stripped — the grid holds post-render characters). `none` is the
 * false-positive guard: e.g. require the approval keyword but forbid a token
 * that only appears in the agent's non-blocking output.
 */
export interface ApprovalRule {
  /** Stable identifier, surfaced in errors and useful when debugging a match. */
  id: string;
  /** Patterns that must ALL be present. */
  all?: string[];
  /** Patterns of which at least ONE must be present (ignored when empty). */
  any?: string[];
  /** Patterns none of which may be present (false-positive guard). */
  none?: string[];
}

/** A per-agent rule set. `approval` matches if ANY of its rules match. */
export interface AgentRulePack {
  /** Agent kind this pack recognizes, matching `AGENTS[...].kind`. */
  agentKind: string;
  /** Approval rules; the pack matches when any single rule matches. */
  approval: ApprovalRule[];
}

/** Compiled counterpart of {@link ApprovalRule} — regexes pre-built once. */
export interface CompiledRule {
  id: string;
  all: RegExp[];
  any: RegExp[];
  none: RegExp[];
}

/** Compiled counterpart of {@link AgentRulePack}. */
export interface CompiledPack {
  agentKind: string;
  approval: CompiledRule[];
}

/** Thrown by {@link compileRulePacks} when a pack is structurally invalid or a
 *  pattern fails to compile. Malformed packs fail LOUDLY (plan §Phase 3 / T3.1
 *  acceptance) rather than silently recognizing nothing. */
export class RulePackError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RulePackError';
  }
}

function compilePatterns(
  patterns: unknown,
  where: string,
): RegExp[] {
  if (patterns === undefined) return [];
  if (!Array.isArray(patterns)) {
    throw new RulePackError(`${where} must be an array of pattern strings`);
  }
  return patterns.map((p, i) => {
    if (typeof p !== 'string' || p.length === 0) {
      throw new RulePackError(`${where}[${i}] must be a non-empty string`);
    }
    try {
      // Case-insensitive: agent prompts vary casing ("Allow"/"allow"), and the
      // grid text is authored, not normalized.
      return new RegExp(p, 'i');
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      throw new RulePackError(`${where}[${i}] is not a valid regex: ${detail}`);
    }
  });
}

function compileRule(rule: unknown, agentKind: string, index: number): CompiledRule {
  const where = `pack "${agentKind}" approval[${index}]`;
  if (typeof rule !== 'object' || rule === null) {
    throw new RulePackError(`${where} must be an object`);
  }
  const r = rule as Record<string, unknown>;
  if (typeof r.id !== 'string' || r.id.length === 0) {
    throw new RulePackError(`${where} is missing a non-empty "id"`);
  }
  const all = compilePatterns(r.all, `${where}.all`);
  const any = compilePatterns(r.any, `${where}.any`);
  const none = compilePatterns(r.none, `${where}.none`);
  if (all.length === 0 && any.length === 0) {
    // A rule that constrains nothing matches every frame — the exact
    // false-positive the "never guess approval from silence" promise forbids.
    throw new RulePackError(
      `${where} ("${r.id}") must set at least one of "all"/"any"`,
    );
  }
  return { id: r.id, all, any, none };
}

/**
 * Validate and compile raw rule packs into the matcher's runtime form, keyed by
 * `agentKind`. Throws {@link RulePackError} on any structural problem or bad
 * regex (T3.1: "malformed pack fails loudly"). Re-callable with a freshly
 * fetched pack set for hot-reload (plan §7.3).
 */
export function compileRulePacks(
  packs: readonly AgentRulePack[],
): Map<string, CompiledPack> {
  if (!Array.isArray(packs)) {
    throw new RulePackError('rule packs must be an array');
  }
  const out = new Map<string, CompiledPack>();
  packs.forEach((pack, i) => {
    if (typeof pack !== 'object' || pack === null) {
      throw new RulePackError(`rule pack [${i}] must be an object`);
    }
    const p = pack as Record<string, unknown>;
    if (typeof p.agentKind !== 'string' || p.agentKind.length === 0) {
      throw new RulePackError(`rule pack [${i}] is missing a non-empty "agentKind"`);
    }
    if (out.has(p.agentKind)) {
      throw new RulePackError(`duplicate rule pack for agentKind "${p.agentKind}"`);
    }
    if (!Array.isArray(p.approval) || p.approval.length === 0) {
      throw new RulePackError(
        `pack "${p.agentKind}" must have a non-empty "approval" rule array`,
      );
    }
    const approval = p.approval.map((rule, j) => compileRule(rule, p.agentKind as string, j));
    out.set(p.agentKind, { agentKind: p.agentKind, approval });
  });
  return out;
}

/**
 * Bundled default packs. These are tuning targets, not gospel — agent approval
 * UIs drift, and the whole point of the data format is that a pattern can be
 * corrected without an engine change. Claude is intentionally absent: it
 * self-reports approval precisely via Phase 2 hooks, so on-screen guessing
 * would only add noise.
 *
 * Patterns lean on the stable, human-readable prompt text each CLI prints when
 * it blocks on a permission/confirmation gate, guarded by `none` where a
 * keyword also appears in non-blocking chatter.
 */
export const DEFAULT_RULE_PACKS: readonly AgentRulePack[] = [
  {
    agentKind: 'codex',
    approval: [
      {
        id: 'codex-allow-command',
        all: ['allow'],
        any: ['run this command', 'execute', 'apply this', 'proceed'],
      },
      {
        id: 'codex-yes-no-gate',
        any: ['\\[y/n\\]', '\\(y/n\\)', 'yes / no', 'approve\\b.*\\bdeny'],
      },
    ],
  },
  {
    agentKind: 'opencode',
    approval: [
      {
        id: 'opencode-permission',
        all: ['permission'],
        any: ['allow', 'grant', 'approve'],
      },
      {
        id: 'opencode-confirm',
        any: ['do you want to proceed', 'confirm to continue', 'allow this action'],
      },
    ],
  },
];

/** The compiled default packs — the recognizer's out-of-the-box configuration.
 *  Compiled once at module load; a bad default pack fails the build's test run
 *  loudly rather than shipping a silently-broken recognizer. */
export const DEFAULT_COMPILED_PACKS: Map<string, CompiledPack> =
  compileRulePacks(DEFAULT_RULE_PACKS);
