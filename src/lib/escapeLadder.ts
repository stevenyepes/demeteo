import type { AppView, Provider } from '../types';

/**
 * Minimal UIState slice consumed by `pickEscapeAction`. The helper
 * only needs the open flags (and the editing-provider boolean), so
 * the slice stays narrow and the helper remains decoupled from the
 * full UIState shape. Mirrors the relevant fields of
 * `src/context/UIStateContext.tsx`.
 */
export interface UIStateSlice {
  /** Overlays that registered themselves via `useOverlay`, newest last. */
  overlays: string[];
  commandPaletteOpen: boolean;
  docsPanelOpen: boolean;
  isConnectModalOpen: boolean;
  editingProvider: Provider | null;
  startFeatureOpen: boolean;
}

/**
 * Discriminated union of every state mutation a single Escape press
 * can perform. The caller (AppInner) translates each variant into a
 * concrete `uiDispatch` / `navigate` / `goBack` call.
 */
export type EscapeAction =
  | { type: 'close-command-palette' }
  | { type: 'close-docs-panel' }
  | { type: 'close-connect-modal' }
  | { type: 'close-start-feature' }
  | { type: 'close-gate-view'; featureId: string; featureTitle: string }
  /** A registered overlay is on screen and owns its own dismissal, so the
   *  global handler does nothing at all. Distinct from `navigate-back` and
   *  from the named rungs: there is no dispatch to make here — acting would be
   *  the bug (audit F35), not the fix. */
  | { type: 'overlay-owns-dismissal' }
  | { type: 'navigate-back' };

/**
 * What a Back gesture should do right now — `Escape`, the mouse back button and
 * `Alt+←` all ask this, so the three cannot drift apart.
 *
 * Priority order (topmost first, per the implementation spec AC-3):
 *   1. command palette     (ui.commandPaletteOpen)
 *   2. docs panel          (ui.docsPanelOpen)
 *   3. provider connect    (ui.isConnectModalOpen || ui.editingProvider)
 *   4. start-feature modal (ui.startFeatureOpen)
 *   5. gate view overlay   (view.kind === 'detail' && view.gateStepExecutionId)
 *   6. any registered overlay (ui.overlays) — stand down, it dismisses itself
 *   7. fallback            (navigate back)
 *
 * Rung 6 is the general case and the other five are older, named ones that
 * predate it; it sits below them so those keep the explicit dispatch they
 * already had. It closes audit F35: before it, every dialog outside `UIState`
 * fell straight through to `navigate-back`, so pressing Escape to dismiss an
 * attachment preview or the ticket editor *also* moved the view underneath.
 * The fix is a registry rather than a sixth flag because a flag list goes stale
 * silently — the next dialog added without one reopens the same bug.
 *
 * This lives outside `App.tsx` so a component *inside* the tree App renders can
 * consult the ladder without an import cycle — see `hasEscapeOverlay`.
 */
export function pickEscapeAction(ui: UIStateSlice, view: AppView): EscapeAction {
  if (ui.commandPaletteOpen) return { type: 'close-command-palette' };
  if (ui.docsPanelOpen) return { type: 'close-docs-panel' };
  if (ui.isConnectModalOpen || ui.editingProvider !== null) return { type: 'close-connect-modal' };
  if (ui.startFeatureOpen) return { type: 'close-start-feature' };
  if (view.kind === 'detail' && view.gateStepExecutionId) {
    return {
      type: 'close-gate-view',
      featureId: view.featureId,
      featureTitle: view.featureTitle,
    };
  }
  if (ui.overlays.length > 0) return { type: 'overlay-owns-dismissal' };
  return { type: 'navigate-back' };
}

/**
 * True when some layer above the base view currently owns the gesture.
 *
 * Derived from `pickEscapeAction` rather than from its own list of flags: a
 * header popover that swallows Escape has to know when it is *not* the topmost
 * thing on screen, and a second copy of the ladder's conditions would go stale
 * the first time a rung is added — silently, because the symptom is a key that
 * stops working, not a type error.
 */
export function hasEscapeOverlay(ui: UIStateSlice, view: AppView): boolean {
  return pickEscapeAction(ui, view).type !== 'navigate-back';
}
