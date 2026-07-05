import type { ReactNode } from 'react';

/**
 * Coarse priority bucket for an overlay entry. The tier alone is enough to
 * rank most overlays; the numeric `priority` field breaks ties inside a tier.
 *
 * Higher tier = closer to the user. AC-3 requires that the tier ordering
 * outranks insertion order: a `toast` pushed after a `modal` still renders
 * behind the modal until the modal is dismissed.
 */
export type OverlayPriorityTier =
  | 'gate'
  | 'modal'
  | 'palette'
  | 'drawer'
  | 'toast';

/**
 * One registered overlay in the stack. Pure data — the React side is
 * responsibility of the caller; this file only owns ordering + identity.
 */
export interface OverlayEntry {
  /** Stable, unique id (caller-supplied or auto-generated via {@link generateOverlayId}). */
  readonly id: string;
  /** Coarse priority bucket. */
  readonly tier: OverlayPriorityTier;
  /** Fine-grained priority within a tier; higher wins. */
  readonly priority: number;
  /** Unix ms when the entry was pushed. Tie-breaker — older sorts lower. */
  readonly createdAt: number;
  /** Optional React content rendered inside the overlay surface. */
  readonly content?: ReactNode;
  /** Called by the global Escape handler before dismissal. */
  readonly onEscape?: () => void;
  /** If `false`, the global handler invokes `onEscape` but skips auto-pop. */
  readonly dismissOnEscape?: boolean;
  /** If `true`, focus returns to the element that was active before this overlay opened. */
  readonly restoreFocus?: boolean;
  /** Human-readable label, used for stable id generation and DevTools. */
  readonly label?: string;
}

/** Immutable view of the stack at one moment in time. */
export interface OverlayStack {
  readonly entries: ReadonlyArray<OverlayEntry>;
}

export type OverlayStackAction =
  | { type: 'PUSH'; entry: OverlayEntry }
  | { type: 'POP'; id: string }
  | { type: 'REPLACE'; entry: OverlayEntry };

/** Numeric rank per tier; used by {@link compareOverlayEntries}. */
export const TIER_RANK: Readonly<Record<OverlayPriorityTier, number>> = {
  gate: 4,
  modal: 3,
  palette: 2,
  drawer: 1,
  toast: 0,
};

/** Default tier when the caller doesn't specify one. */
export const DEFAULT_TIER: OverlayPriorityTier = 'modal';

/** Constant empty stack — usable as `useReducer` initial value. */
export const EMPTY_STACK: OverlayStack = Object.freeze({
  entries: Object.freeze([]) as ReadonlyArray<OverlayEntry>,
}) as OverlayStack;

/**
 * Comparator used to sort entries highest-priority-first.
 *
 * Order:
 *   1. tier rank desc (gate > modal > palette > drawer > toast)
 *   2. numeric priority desc (higher wins inside a tier)
 *   3. createdAt desc (newer pushes surface above older ones)
 */
export function compareOverlayEntries(a: OverlayEntry, b: OverlayEntry): number {
  const tierA = TIER_RANK[a.tier] ?? 0;
  const tierB = TIER_RANK[b.tier] ?? 0;
  if (tierA !== tierB) return tierB - tierA;
  if (a.priority !== b.priority) return b.priority - a.priority;
  if (a.createdAt !== b.createdAt) return b.createdAt - a.createdAt;
  if (a.id !== b.id) return a.id < b.id ? -1 : 1;
  return 0;
}

/** Stable-friendly sort that returns a fresh array. */
export function sortOverlayEntries(
  entries: ReadonlyArray<OverlayEntry>,
): ReadonlyArray<OverlayEntry> {
  return [...entries].sort(compareOverlayEntries);
}

/**
 * Pure reducer — never mutates input.
 *
 * - `PUSH` rejects duplicate ids (silently ignored — push is idempotent on id).
 * - `POP` removes the first entry matching the id; no-op if absent.
 * - `REPLACE` updates an entry in place (preserving its original `createdAt`),
 *   so the new payload doesn't disturb ordering relative to its peers.
 */
export function overlayStackReducer(
  state: OverlayStack,
  action: OverlayStackAction,
): OverlayStack {
  switch (action.type) {
    case 'PUSH': {
      if (state.entries.some((e) => e.id === action.entry.id)) return state;
      return { entries: sortOverlayEntries([...state.entries, action.entry]) };
    }
    case 'POP': {
      if (!state.entries.some((e) => e.id === action.id)) return state;
      return { entries: state.entries.filter((e) => e.id !== action.id) };
    }
    case 'REPLACE': {
      const idx = state.entries.findIndex((e) => e.id === action.entry.id);
      if (idx === -1) {
        return { entries: sortOverlayEntries([...state.entries, action.entry]) };
      }
      // Preserve the original `createdAt` to keep ordering stable across
      // replacements — a re-render with new `onEscape` shouldn't reshuffle.
      const preserved: OverlayEntry = {
        ...action.entry,
        createdAt: state.entries[idx].createdAt,
      };
      const next = [...state.entries];
      next[idx] = preserved;
      return { entries: sortOverlayEntries(next) };
    }
    default:
      return state;
  }
}

let overlayCounter = 0;

/**
 * Generates a stable, monotonic id. Independent of `Date.now()` so consecutive
 * calls inside the same millisecond remain unique. The optional `prefix`
 * surfaces in DevTools and stack traces.
 */
export function generateOverlayId(prefix: string = 'overlay'): string {
  overlayCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${overlayCounter.toString(36)}`;
}

/** Caller-facing push options — `id` and `createdAt` are filled in for you. */
export interface PushOptions {
  readonly id?: string;
  readonly tier?: OverlayPriorityTier;
  readonly priority?: number;
  readonly content?: ReactNode;
  readonly onEscape?: () => void;
  readonly dismissOnEscape?: boolean;
  readonly restoreFocus?: boolean;
  readonly label?: string;
  /**
   * Optional override for `Date.now()`. Tests use this to control ordering;
   * production callers should leave it unset.
   */
  readonly createdAt?: number;
}

export interface PushResult {
  readonly state: OverlayStack;
  readonly entry: OverlayEntry;
}

/**
 * Wraps the reducer for the common case: caller supplies options, gets back
 * the new state and the entry that was pushed. If `id` is omitted, one is
 * generated via {@link generateOverlayId}.
 */
export function pushOverlay(
  state: OverlayStack,
  options: PushOptions = {},
): PushResult {
  const id = options.id ?? generateOverlayId(options.label);
  const entry: OverlayEntry = {
    id,
    tier: options.tier ?? DEFAULT_TIER,
    priority: options.priority ?? 0,
    createdAt: options.createdAt ?? Date.now(),
    content: options.content,
    onEscape: options.onEscape,
    dismissOnEscape: options.dismissOnEscape ?? true,
    restoreFocus: options.restoreFocus ?? true,
    label: options.label,
  };
  return { state: overlayStackReducer(state, { type: 'PUSH', entry }), entry };
}

/** Pop by id; no-op when the id isn't in the stack. */
export function popOverlay(state: OverlayStack, id: string): OverlayStack {
  return overlayStackReducer(state, { type: 'POP', id });
}

/**
 * Replace fields for an existing entry. Preserves `createdAt` so the entry
 * keeps its slot in the sort order unless `tier`/`priority` change.
 * Returns the input state unchanged when the id is absent.
 */
export function replaceOverlay(
  state: OverlayStack,
  id: string,
  options: Omit<PushOptions, 'id'> = {},
): OverlayStack {
  const idx = state.entries.findIndex((e) => e.id === id);
  if (idx === -1) return state;
  const existing = state.entries[idx];
  const entry: OverlayEntry = {
    id,
    tier: options.tier ?? existing.tier,
    priority: options.priority ?? existing.priority,
    createdAt: existing.createdAt,
    content: options.content ?? existing.content,
    onEscape: options.onEscape ?? existing.onEscape,
    dismissOnEscape: options.dismissOnEscape ?? existing.dismissOnEscape,
    restoreFocus: options.restoreFocus ?? existing.restoreFocus,
    label: options.label ?? existing.label,
  };
  return overlayStackReducer(state, { type: 'REPLACE', entry });
}

/** Returns the topmost entry (position 0 after sorting) or undefined. */
export function topOverlay(state: OverlayStack): OverlayEntry | undefined {
  return state.entries[0];
}

/** Predicate — is `id` currently in the stack? */
export function hasOverlay(state: OverlayStack, id: string): boolean {
  return state.entries.some((e) => e.id === id);
}

/**
 * Stably removes a list of ids in one pass. Used when an overlay tree
 * unmounts in bulk (e.g. an error toast group).
 */
export function popMany(
  state: OverlayStack,
  ids: ReadonlyArray<string>,
): OverlayStack {
  const drop = new Set(ids);
  if (drop.size === 0) return state;
  const next = state.entries.filter((e) => !drop.has(e.id));
  return next.length === state.entries.length
    ? state
    : { entries: next };
}
