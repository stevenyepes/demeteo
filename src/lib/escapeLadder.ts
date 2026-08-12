import type { AppView, Provider } from '../types';

/**
 * Minimal UIState slice consumed by `pickEscapeAction`. The helper
 * only needs the open flags (and the editing-provider boolean), so
 * the slice stays narrow and the helper remains decoupled from the
 * full UIState shape. Mirrors the relevant fields of
 * `src/context/UIStateContext.tsx`.
 */
export interface UIStateSlice {
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
  | { type: 'navigate-back' };

/**
 * Decide which overlay (if any) a single Escape press should close.
 *
 * Priority order (topmost first, per the implementation spec AC-3):
 *   1. command palette     (ui.commandPaletteOpen)
 *   2. docs panel          (ui.docsPanelOpen)
 *   3. provider connect    (ui.isConnectModalOpen || ui.editingProvider)
 *   4. start-feature modal (ui.startFeatureOpen)
 *   5. gate view overlay   (view.kind === 'detail' && view.gateStepExecutionId)
 *   6. fallback            (navigate back)
 *
 * Per-modal ESC handlers in `CommandPalette`, `StartFeatureModal`,
 * `DocsPanel`, `EnvModal`, `GateView`, etc. are expected to call
 * `event.stopPropagation()` so the global hook only fires once. The
 * notification-bell popover is owned by `NotificationBell` and
 * dismisses itself on the same keypress; the prompt dialog
 * (`FeatureDetail`'s local modal) is handled the same way.
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
  return { type: 'navigate-back' };
}

/**
 * True when some layer above the base view currently owns Escape.
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
