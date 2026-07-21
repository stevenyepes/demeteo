// Terminal-activity Phase 3 — presence/confirmation debounce (T3.3).
//
// The recognizer (`recognizerTick`) answers "is an approval prompt on screen
// *this frame*?" — but a single transient frame (a half-drawn prompt, a redraw
// that momentarily blanks the tail, the reporter's own bytes) must not flip the
// user-visible "needs a decision" mark. This debouncer commits a change only
// after the opposite reading has persisted across a threshold of consecutive
// scans, so one-frame blips are absorbed (plan §Phase 3: "a small
// presence/confirmation debounce prevents flap on transient frames").
//
// Pure and framework-free: the caller drives `observe(present)` at whatever
// cadence it scans (throttled to render-idle) and, on a non-null return, reports
// the newly-committed state to the backend. Separate enter/exit thresholds let
// asserting an approval be as eager or as cautious as retracting it.

export interface DebounceConfig {
  /** Consecutive matching scans required before committing `awaiting_approval`
   *  (asserting). Higher = fewer false positives, slightly slower to light up. */
  enterFrames: number;
  /** Consecutive non-matching scans required before retracting. Kept modest so
   *  the mark clears promptly once the user acts on the prompt. */
  exitFrames: number;
}

/** Default thresholds. Two frames each: absorbs a single transient frame in
 *  either direction while keeping the mark responsive at the recognizer's
 *  render-idle cadence. */
export const DEFAULT_DEBOUNCE: DebounceConfig = { enterFrames: 2, exitFrames: 2 };

/**
 * Debounces the recognizer's per-frame boolean into a committed present/absent
 * state. `observe(present)` returns the newly-committed value on a transition,
 * or `null` when nothing changed (steady state, or evidence still accumulating).
 *
 * The committed state starts absent (`false`): a session is never assumed to be
 * awaiting approval until the recognizer has actually and repeatedly seen it.
 */
export class ScreenApprovalDebouncer {
  private committed = false;
  /** Consecutive scans that disagree with `committed`, i.e. evidence toward a
   *  flip. Reset whenever a scan agrees with the committed state. */
  private streak = 0;

  constructor(private readonly config: DebounceConfig = DEFAULT_DEBOUNCE) {}

  /** The current committed (debounced) state. */
  get state(): boolean {
    return this.committed;
  }

  /**
   * Feed one recognizer reading. Returns the new committed state when this
   * observation tips a transition, else `null`.
   */
  observe(present: boolean): boolean | null {
    if (present === this.committed) {
      // Agrees with the committed state — cancel any pending flip.
      this.streak = 0;
      return null;
    }
    this.streak += 1;
    const threshold = present ? this.config.enterFrames : this.config.exitFrames;
    if (this.streak >= threshold) {
      this.committed = present;
      this.streak = 0;
      return present;
    }
    return null;
  }

  /** Force the committed state without debounce (e.g. on teardown to retract a
   *  latched approval when a surface unmounts). Returns the new state if it
   *  changed, else `null`. */
  reset(to = false): boolean | null {
    this.streak = 0;
    if (this.committed === to) return null;
    this.committed = to;
    return to;
  }
}
