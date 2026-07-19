// Persistence for the `+ New` launcher's "Recent" strip — the last handful
// of machine × runtime launches, surfaced as one-tap chips so re-opening a
// session you just used is a single click rather than a scroll through the
// full machine → runtime picker. Stored in localStorage (a pure UI
// convenience; no backend round-trip and safe to lose).

const STORAGE_KEY = 'demeteo.terminal.recents';
/** How many launches we remember. A few more than we render so a removed
 *  machine dropping out still leaves the strip populated. */
const MAX_STORED = 6;
/** How many chips the menu renders. */
export const RECENTS_SHOWN = 4;

/** One remembered launch: a machine plus the runtime (a bare shell when
 *  `agentKind` is null, otherwise a coding agent). */
export interface TerminalRecent {
  machineId: string;
  /** Label captured at launch time; re-resolved against the live machine
   *  list when rendered, so a rename shows the current name. */
  machineLabel: string;
  agentKind: string | null;
}

/** Stable identity of a launch — a machine + runtime pair. Two launches with
 *  the same pair collapse to the most recent one. */
function keyOf(machineId: string, agentKind: string | null): string {
  return `${machineId}::${agentKind ?? ''}`;
}

function safeParse(raw: string | null): TerminalRecent[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (r): r is TerminalRecent =>
          !!r && typeof r.machineId === 'string' && typeof r.machineLabel === 'string',
      )
      .map((r) => ({
        machineId: r.machineId,
        machineLabel: r.machineLabel,
        agentKind: typeof r.agentKind === 'string' ? r.agentKind : null,
      }));
  } catch {
    return [];
  }
}

/** Read the stored launches, most-recent first. Never throws. */
export function loadRecents(): TerminalRecent[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    return safeParse(localStorage.getItem(STORAGE_KEY));
  } catch {
    return [];
  }
}

/**
 * Record a launch at the front of the list, collapsing any earlier launch of
 * the same machine × runtime pair, and return the updated list (most-recent
 * first) so the caller can re-render without a second read.
 */
export function recordRecent(entry: TerminalRecent): TerminalRecent[] {
  const normalized: TerminalRecent = {
    machineId: entry.machineId,
    machineLabel: entry.machineLabel,
    agentKind: entry.agentKind ?? null,
  };
  const k = keyOf(normalized.machineId, normalized.agentKind);
  const next = [normalized, ...loadRecents().filter((r) => keyOf(r.machineId, r.agentKind) !== k)].slice(
    0,
    MAX_STORED,
  );
  if (typeof localStorage !== 'undefined') {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    } catch {
      /* storage full or unavailable — the strip is best-effort */
    }
  }
  return next;
}
