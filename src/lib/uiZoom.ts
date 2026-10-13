/**
 * Whole-window zoom: the decisions, with the one IPC call they end in.
 *
 * This replaces Tauri's `zoomHotkeysEnabled`, which is off in
 * `src-tauri/tauri.conf.json` and has to stay off while this module exists —
 * the two would both answer the same keystroke. Its polyfill moved 20% per
 * `wheel` event with Ctrl held, and a trackpad pinch *is* a stream of those
 * events, dozens a second: one pinch crossed the whole 20%–1000% range, so no
 * level in between could be landed on. It also kept the level in a script
 * variable, so every launch opened at 100%, and it only existed on macOS and
 * Linux — Windows got WebView2's native control, with different steps.
 *
 * What stands in for it is one implementation for all three, shaped like an
 * editor's rather than a browser's: a fixed ladder of levels, one rung per
 * keypress, the choice stored (`uiZoomPref`).
 *
 * The pinch stays, tamed rather than removed, by two limits that do different
 * jobs. Travel is banked until it amounts to a deliberate movement, so the
 * few pixels of ctrl-wheel a resting thumb produces move nothing. And a rung
 * cannot follow another inside `WHEEL_COOLDOWN_MS`, which is the limit that
 * still holds if a webview reports pinch travel on a scale nobody measured —
 * the failure being replaced was exactly an unbounded rate.
 */

import { getCurrentWebview } from '@tauri-apps/api/webview';

import type { KeyboardEventLike } from './shortcuts';

/** Browser-style rungs: fine around 100%, where the choice is actually made,
 *  and coarse at the ends. A constant ratio (VS Code's 1.2ⁿ) has no rung
 *  between 100% and 120%, which is the step most people want. */
export const UI_ZOOM_LEVELS: readonly number[] = [
  0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2,
];

export const DEFAULT_UI_ZOOM = 1;

export type UiZoomAction = 'in' | 'out' | 'reset';

const UI_ZOOM_MIN = UI_ZOOM_LEVELS[0];
const UI_ZOOM_MAX = UI_ZOOM_LEVELS[UI_ZOOM_LEVELS.length - 1];

/** The rung nearest `zoom`. Everything that enters from outside — the store,
 *  a caller — is snapped through this, so the ladder is the only set of values
 *  the window is ever at and a step from any of them lands on a neighbour. */
export function nearestUiZoom(zoom: number): number {
  let best = DEFAULT_UI_ZOOM;
  for (const level of UI_ZOOM_LEVELS) {
    if (Math.abs(level - zoom) < Math.abs(best - zoom)) best = level;
  }
  return best;
}

export function nextUiZoom(zoom: number, action: UiZoomAction): number {
  if (action === 'reset') return DEFAULT_UI_ZOOM;
  const index = UI_ZOOM_LEVELS.indexOf(nearestUiZoom(zoom));
  const target = action === 'in' ? index + 1 : index - 1;
  return UI_ZOOM_LEVELS[Math.min(UI_ZOOM_LEVELS.length - 1, Math.max(0, target))];
}

/** `undefined` for a stored value this build cannot use — see `uiPrefs.ts`. */
export function decodeUiZoom(raw: string): number | undefined {
  const zoom = Number(raw);
  if (raw.trim() === '' || !Number.isFinite(zoom)) return undefined;
  if (zoom < UI_ZOOM_MIN || zoom > UI_ZOOM_MAX) return undefined;
  return nearestUiZoom(zoom);
}

export function uiZoomPercent(zoom: number): string {
  return `${Math.round(zoom * 100)}%`;
}

/**
 * Which zoom action a keystroke asks for, if any.
 *
 * Shift is not compared, unlike every chord in `shortcuts.ts`: `+` is
 * Shift+`=` on a US layout and unshifted on a German one, and the numpad sends
 * it bare, so "zoom in" is whichever of the two characters arrives. Alt is
 * compared, because AltGr reports as Ctrl+Alt on Windows and several layouts
 * type `-` and `0`-row characters through it.
 */
export function uiZoomKeyAction(event: KeyboardEventLike): UiZoomAction | null {
  if (!(event.metaKey || event.ctrlKey) || event.altKey) return null;
  switch (event.key) {
    case '=':
    case '+':
      return 'in';
    case '-':
    case '_':
      return 'out';
    case '0':
      return 'reset';
    default:
      return null;
  }
}

/** Banked travel that earns one rung. Half a mouse-wheel notch on the
 *  platform with the smallest notch, so a notch is always a step. */
export const WHEEL_STEP_PX = 50;

/** Least time between two rungs from the wheel. */
export const WHEEL_COOLDOWN_MS = 120;

/** A pause this long ends the gesture, and what it had banked goes with it. */
export const WHEEL_IDLE_MS = 300;

export interface WheelGesture {
  /** Signed travel banked toward the next rung; negative zooms in. */
  banked: number;
  /** When the last event of this gesture arrived. */
  lastEventAt: number;
  /** When this gesture last earned a rung. */
  lastStepAt: number;
}

export const IDLE_WHEEL_GESTURE: WheelGesture = {
  banked: 0,
  lastEventAt: Number.NEGATIVE_INFINITY,
  lastStepAt: Number.NEGATIVE_INFINITY,
};

/**
 * Fold one ctrl-wheel event into the gesture. `pixels` is `deltaY` already
 * normalised by `wheelPixels`; `now` is any monotonic millisecond clock.
 *
 * Travel that arrives during the cooldown is dropped, not banked. Banked, a
 * fast pinch would have a rung waiting the instant the cooldown ended and take
 * it on the gesture's last stray pixel — one step further than the fingers
 * asked for, every time.
 */
export function advanceWheelGesture(
  gesture: WheelGesture,
  pixels: number,
  now: number,
): { gesture: WheelGesture; action: UiZoomAction | null } {
  if (now - gesture.lastStepAt < WHEEL_COOLDOWN_MS) {
    return { gesture: { ...gesture, banked: 0, lastEventAt: now }, action: null };
  }
  const continues =
    now - gesture.lastEventAt < WHEEL_IDLE_MS &&
    (gesture.banked === 0 || Math.sign(gesture.banked) === Math.sign(pixels));
  const banked = (continues ? gesture.banked : 0) + pixels;

  if (Math.abs(banked) < WHEEL_STEP_PX) {
    return { gesture: { ...gesture, banked, lastEventAt: now }, action: null };
  }
  return {
    gesture: { banked: 0, lastEventAt: now, lastStepAt: now },
    action: banked < 0 ? 'in' : 'out',
  };
}

/**
 * Put the window at `zoom`. Never rejects: outside a Tauri host there is no
 * webview to address (`vite` alone, a component test), and a zoom that failed
 * to apply leaves the user nothing to act on.
 */
export async function applyUiZoom(zoom: number): Promise<void> {
  try {
    await getCurrentWebview().setZoom(zoom);
  } catch {
    /* see above */
  }
}
