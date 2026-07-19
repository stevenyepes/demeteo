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
 */
const STORAGE_KEY = 'demeteo.terminal.lastSize';

type Size = { cols: number; rows: number };

function loadPersisted(): Size | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<Size>;
    if (
      typeof parsed.cols === 'number' &&
      typeof parsed.rows === 'number' &&
      parsed.cols > 0 &&
      parsed.rows > 0
    ) {
      return { cols: parsed.cols, rows: parsed.rows };
    }
  } catch {
    // localStorage unavailable or malformed value — fall back to the backend default.
  }
  return null;
}

let lastTerminalSize: Size | null = loadPersisted();

/** Record the latest fitted size. Ignores non-positive dimensions. */
export function setLastTerminalSize(cols: number, rows: number): void {
  if (cols > 0 && rows > 0) {
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
