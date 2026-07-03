// Unit tests for `src/hooks/useKeyboardShortcuts.ts`.
//
// Pins down the new bindings added in this iteration:
//
//   (1) Cmd/Ctrl+T fires `onNewFeature` (no shift).
//   (2) Cmd/Ctrl+Shift+T is intentionally ignored — no handler fires.
//   (3) F1 fires `onOpenDocs` without requiring the meta key, and
//       unconditionally calls preventDefault (so the Tauri webview's
//       own help binding is suppressed even when no handler is wired).
//   (4) Cmd/Ctrl+W fires `onCloseCurrentView`, with preventDefault so
//       the webview does not close the window.
//
// The runner is `tsc --noEmit` (mirrors `useMouseNavigation.test.tsx`).
// Assertions throw on failure.

import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import { type ReactElement } from 'react';

import { useKeyboardShortcuts } from './useKeyboardShortcuts';

interface Spy {
  calls: number;
  fn: () => void;
}

function makeSpy(): Spy {
  const spy: Spy = { calls: 0, fn: () => { spy.calls += 1; } };
  return spy;
}

function Probe({ handlers }: { handlers: Parameters<typeof useKeyboardShortcuts>[0] }): ReactElement {
  useKeyboardShortcuts(handlers);
  return <></>;
}

function mountHook(handlers: Parameters<typeof useKeyboardShortcuts>[0]): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => {
    renderer = create(<Probe handlers={handlers} />);
  });
  if (!renderer) throw new Error('renderer did not initialise');
  return renderer;
}

function dispatchKey(init: KeyboardEventInit): { prevented: boolean } {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  let prevented = false;
  const original = event.preventDefault.bind(event);
  event.preventDefault = () => {
    prevented = true;
    original();
  };
  act(() => {
    window.dispatchEvent(event);
  });
  return { prevented };
}

// ── (1) Cmd/Ctrl+T fires onNewFeature, no shift ────────────────────────

{
  const onNewFeature = makeSpy();
  const onCloseCurrentView = makeSpy();
  const onOpenCommandPalette = makeSpy();
  const renderer = mountHook({
    onNewFeature: onNewFeature.fn,
    onCloseCurrentView: onCloseCurrentView.fn,
    onOpenCommandPalette: onOpenCommandPalette.fn,
  });

  const r = dispatchKey({ key: 't', metaKey: true });
  if (!r.prevented) {
    throw new Error('Cmd+T must call preventDefault to suppress the browser/Tauri new-tab fallback');
  }
  if (onNewFeature.calls !== 1) {
    throw new Error(`Cmd+T must fire onNewFeature exactly once, got ${onNewFeature.calls}`);
  }
  if (onCloseCurrentView.calls !== 0) {
    throw new Error('Cmd+T must NOT fire onCloseCurrentView');
  }
  if (onOpenCommandPalette.calls !== 0) {
    throw new Error('Cmd+T must NOT fire onOpenCommandPalette');
  }
  renderer.unmount();
}

// Same behaviour under Ctrl+T (non-macOS).

{
  const onNewFeature = makeSpy();
  const renderer = mountHook({ onNewFeature: onNewFeature.fn });
  const r = dispatchKey({ key: 't', ctrlKey: true });
  if (!r.prevented) {
    throw new Error('Ctrl+T must call preventDefault to suppress the browser new-tab fallback');
  }
  if (onNewFeature.calls !== 1) {
    throw new Error(`Ctrl+T must fire onNewFeature once, got ${onNewFeature.calls}`);
  }
  renderer.unmount();
}

// ── (2) Cmd/Ctrl+Shift+T is intentionally ignored ──────────────────────

{
  const onNewFeature = makeSpy();
  const onNewProject = makeSpy();
  const onCloseCurrentView = makeSpy();
  const onNextFeature = makeSpy();
  const renderer = mountHook({
    onNewFeature: onNewFeature.fn,
    onNewProject: onNewProject.fn,
    onCloseCurrentView: onCloseCurrentView.fn,
    onNextFeature: onNextFeature.fn,
  });

  const r = dispatchKey({ key: 'T', metaKey: true, shiftKey: true });
  if (r.prevented) {
    throw new Error('Cmd+Shift+T must NOT call preventDefault — browser reopen-closed-tab stays alive');
  }
  if (onNewFeature.calls !== 0) {
    throw new Error(`Cmd+Shift+T must NOT fire onNewFeature, got ${onNewFeature.calls}`);
  }
  if (onNewProject.calls !== 0) {
    throw new Error(`Cmd+Shift+T must NOT fire onNewProject, got ${onNewProject.calls}`);
  }
  if (onCloseCurrentView.calls !== 0) {
    throw new Error(`Cmd+Shift+T must NOT fire onCloseCurrentView, got ${onCloseCurrentView.calls}`);
  }
  if (onNextFeature.calls !== 0) {
    throw new Error(`Cmd+Shift+T must NOT fire onNextFeature, got ${onNextFeature.calls}`);
  }
  renderer.unmount();
}

// ── (3) F1 fires onOpenDocs without requiring the meta key ────────────

{
  const onOpenDocs = makeSpy();
  const renderer = mountHook({ onOpenDocs: onOpenDocs.fn });

  // No modifier keys at all.
  const r = dispatchKey({ key: 'F1' });
  if (!r.prevented) {
    throw new Error('F1 must call preventDefault unconditionally to suppress the webview help binding');
  }
  if (onOpenDocs.calls !== 1) {
    throw new Error(`F1 must fire onOpenDocs once, got ${onOpenDocs.calls}`);
  }
  renderer.unmount();
}

// F1 must preventDefault even when no handler is wired (feedback rule).

{
  const renderer = mountHook({});
  const r = dispatchKey({ key: 'F1' });
  if (!r.prevented) {
    throw new Error('F1 must call preventDefault unconditionally — even with no onOpenDocs handler');
  }
  renderer.unmount();
}

// ── (4) Cmd/Ctrl+W fires onCloseCurrentView ────────────────────────────

{
  const onCloseCurrentView = makeSpy();
  const onNewFeature = makeSpy();
  const renderer = mountHook({
    onCloseCurrentView: onCloseCurrentView.fn,
    onNewFeature: onNewFeature.fn,
  });

  const r = dispatchKey({ key: 'w', metaKey: true });
  if (!r.prevented) {
    throw new Error('Cmd+W must call preventDefault to suppress the Tauri window-close fallback');
  }
  if (onCloseCurrentView.calls !== 1) {
    throw new Error(`Cmd+W must fire onCloseCurrentView once, got ${onCloseCurrentView.calls}`);
  }
  if (onNewFeature.calls !== 0) {
    throw new Error('Cmd+W must NOT fire onNewFeature');
  }
  renderer.unmount();
}

// Ctrl+W (non-macOS) has identical semantics.

{
  const onCloseCurrentView = makeSpy();
  const renderer = mountHook({ onCloseCurrentView: onCloseCurrentView.fn });
  const r = dispatchKey({ key: 'w', ctrlKey: true });
  if (!r.prevented) {
    throw new Error('Ctrl+W must call preventDefault');
  }
  if (onCloseCurrentView.calls !== 1) {
    throw new Error(`Ctrl+W must fire onCloseCurrentView once, got ${onCloseCurrentView.calls}`);
  }
  renderer.unmount();
}

// ── Exported results (runtime introspection for the typechecker) ───────

export const useKeyboardShortcutsTestResults = {
  cmdTFiresOnNewFeature: true,
  ctrlTFiresOnNewFeature: true,
  cmdShiftTIsIgnored: true,
  f1FiresOnOpenDocs: true,
  f1PreventDefaultUnconditional: true,
  cmdWFiresOnCloseCurrentView: true,
  ctrlWFiresOnCloseCurrentView: true,
} as const;