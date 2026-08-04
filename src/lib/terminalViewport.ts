/**
 * Shared cache of the most recent *fitted* terminal size (cols × rows), as
 * measured by a mounted `TerminalSurface` after `FitAddon.fit()`.
 *
 * Why a module-level singleton: when `open()` starts a new session the target
 * surface has not mounted yet, so it cannot measure itself. Every terminal in
 * the full-page Terminals view shares roughly the same viewport, so the last
 * size any surface reported is an excellent estimate for the next one. Passing
 * it to `start_terminal_session` lets the shell draw its *first* prompt at the
 * real width — and, crucially, lets the surface recognise that its own fit
 * matches the spawn size and therefore SKIP the post-attach resize. That
 * skipped resize matters: a `SIGWINCH` arriving during Powerlevel10k's
 * instant/transient-prompt startup corrupts the prompt redraw and duplicates
 * the command line. Spawn-at-the-right-size + skip-the-no-op-resize keeps the
 * whole startup `SIGWINCH`-free.
 *
 * The last value is also persisted to `localStorage` so the *first* terminal
 * opened after an app restart still spawns at the right width (the in-memory
 * singleton resets on restart; the persisted value survives).
 *
 * Why a plausibility floor guards every write *and* every read: a surface that
 * measures itself from inside a `display:none` subtree still measures. There,
 * `getComputedStyle` hands back the *computed* value of a `w-full`/`h-full`
 * box — the literal string `"100%"` — which FitAddon's `proposeDimensions()`
 * `parseInt`s into 100 pixels. At `fontSize: 13` that is a perfectly positive
 * 11 × 5, so a `> 0` test admits it. One such fit would otherwise reach the
 * singleton, `localStorage`, and every subsequent session spawn — including
 * after a restart, which is why the persisted value is re-validated on read
 * rather than trusted.
 */
const STORAGE_KEY = 'demeteo.terminal.lastSize';

type Size = { cols: number; rows: number };

/** Smallest (cols, rows) a genuinely-laid-out Demeteo terminal can fit to. */
export const MIN_PLAUSIBLE_COLS = 20;
export const MIN_PLAUSIBLE_ROWS = 5;

/**
 * The single rule for "is this a real fit?" — synchronous and DOM-free so it
 * is reachable from anywhere a size is about to be trusted.
 *
 * Stated as two positive bounds rather than a negated comparison: `NaN` fails
 * both, where `!(cols < MIN)` would accept it.
 */
export function isPlausibleTerminalSize(cols: number, rows: number): boolean {
  return cols >= MIN_PLAUSIBLE_COLS && rows >= MIN_PLAUSIBLE_ROWS;
}

/**
 * True when `el` participates in layout, i.e. is not inside a `display:none`
 * subtree — the precondition for any measurement of it meaning anything.
 *
 * `offsetParent` is the cheap probe and answers for the common case; it is
 * also `null` for `position: fixed`, so a non-empty border box is accepted as
 * the fallback. Neither costs a `getComputedStyle`.
 */
export function hasLayoutBox(el: HTMLElement | null): boolean {
  if (!el) return false;
  if (el.offsetParent !== null) return true;
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0;
}

function loadPersisted(): Size | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Size>;
    if (
      typeof parsed.cols === 'number' &&
      typeof parsed.rows === 'number' &&
      isPlausibleTerminalSize(parsed.cols, parsed.rows)
    ) {
      return { cols: parsed.cols, rows: parsed.rows };
    }
  } catch {
    // localStorage unavailable or malformed value — fall back to the backend default.
  }
  return null;
}

let lastTerminalSize: Size | null = loadPersisted();

/** Record the latest fitted size. Ignores anything below the plausibility floor. */
export function setLastTerminalSize(cols: number, rows: number): void {
  if (isPlausibleTerminalSize(cols, rows)) {
    lastTerminalSize = { cols, rows };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(lastTerminalSize));
    } catch {
      // Persistence is best-effort; the in-memory value still serves this session.
    }
  }
}

/** Latest fitted size, or `null` if none has ever been measured/persisted. */
export function getLastTerminalSize(): Size | null {
  return lastTerminalSize;
}
