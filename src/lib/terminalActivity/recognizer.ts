// Terminal-activity Phase 3 — recognition engine (T3.2).
//
// Pure matching + rendered-grid extraction, decoupled from any particular
// buffer source. The scan driver (`recognizerTick`) reads the bottom rows from
// an injected source and returns whether a known approval prompt is on screen.
// Because the source is abstract, the same engine serves the focused
// `TerminalSurface` today and headless per-tab buffers later ("Option B") with
// no change here.
//
// Strict approval-only (plan §Phase 3): a match only ever promotes
// `awaiting_input → awaiting_approval`. Silence NEVER yields approval — the
// engine reports `false` for an empty/blank screen, so "needs a decision" is
// only ever claimed when a real prompt is rendered.

import type { CompiledPack, CompiledRule } from './rulePacks';

/** How many bottom rows of the rendered grid the recognizer scans. Approval
 *  prompts render near the cursor at the foot of the screen; scanning only the
 *  tail keeps matching cheap and avoids matching stale prompt text that has
 *  scrolled up into history. */
export const DEFAULT_SCAN_ROWS = 14;

/** One rendered line — the minimum xterm buffer-line surface the reader needs.
 *  Declared structurally so tests (and the future headless source) don't depend
 *  on importing the full `@xterm/xterm` types. */
export interface BufferLineLike {
  translateToString(trimRight?: boolean): string;
}

/** The minimum of an xterm `Terminal` the row reader touches. `buffer.active`
 *  is the *rendered* buffer; `baseY` is the first visible row's index within
 *  it, so `[baseY, baseY + rows)` is exactly the on-screen viewport — never the
 *  scrollback the user can scroll up into (plan §Phase 3: "never the
 *  scrollback"). */
export interface TerminalLike {
  rows: number;
  buffer: {
    active: {
      baseY: number;
      getLine(y: number): BufferLineLike | undefined;
    };
  };
}

/**
 * Read the bottom `n` rows of the *rendered* viewport as trimmed strings,
 * top-to-bottom. Clamped to the viewport so a short screen (or `n` larger than
 * the row count) simply yields the visible rows. Never reads scrollback: it
 * starts at `baseY` (the top of what's on screen), so text the user scrolled
 * past cannot re-trigger a match.
 */
export function readBottomRows(term: TerminalLike, n: number = DEFAULT_SCAN_ROWS): string[] {
  const rows = Math.max(0, term.rows | 0);
  if (rows === 0 || n <= 0) return [];
  const count = Math.min(n, rows);
  const buf = term.buffer.active;
  const firstVisible = buf.baseY;
  const start = firstVisible + (rows - count);
  const out: string[] = [];
  for (let i = 0; i < count; i++) {
    const line = buf.getLine(start + i);
    out.push(line ? line.translateToString(true) : '');
  }
  return out;
}

/** Whether a single compiled rule matches the (already joined + lowercased-by-
 *  regex-flag) screen text. `all` ⇒ every pattern present; `any` ⇒ at least one
 *  present (skipped when empty); `none` ⇒ no forbidden pattern present. */
function ruleMatches(rule: CompiledRule, text: string): boolean {
  for (const re of rule.all) {
    if (!re.test(text)) return false;
  }
  if (rule.any.length > 0 && !rule.any.some((re) => re.test(text))) {
    return false;
  }
  for (const re of rule.none) {
    if (re.test(text)) return false;
  }
  return true;
}

/**
 * Does any rule in `pack` match these rendered rows? Rows are joined with
 * newlines so a pattern can span the label/prompt split an agent renders across
 * two lines. A blank/empty screen matches nothing (strict approval-only).
 */
export function matchesApproval(rows: readonly string[], pack: CompiledPack): boolean {
  // Blank tail ⇒ no prompt ⇒ never approval. Guard explicitly so an all-empty
  // scan can't match a pathological rule.
  const text = rows.join('\n');
  if (text.trim().length === 0) return false;
  return pack.approval.some((rule) => ruleMatches(rule, text));
}

/**
 * One recognition scan against an abstract buffer source. Returns `true` when
 * the agent's approval prompt is currently rendered, `false` otherwise (no
 * pack for the agent ⇒ `false`, never a guess). Pure and synchronous — the
 * caller decides cadence (throttled to render-idle) and debounce (T3.3); this
 * is only the "is a prompt on screen right now" primitive.
 */
export function recognizerTick(
  getRows: () => readonly string[],
  pack: CompiledPack | undefined,
): boolean {
  if (!pack) return false;
  return matchesApproval(getRows(), pack);
}
