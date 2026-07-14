// Unit tests for `src/hooks/useKeyboardShortcuts.ts`.
//
// The through-line: every binding the app claims must also call
// `preventDefault()`, otherwise the Tauri webview runs its own fallback (new
// tab, window close, help overlay) on top of ours. The one deliberate
// exception is Cmd/Ctrl+Shift+T, which we leave to the webview.

import { act, render } from '@testing-library/react';
import { type ReactElement } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { useKeyboardShortcuts } from './useKeyboardShortcuts';

type Handlers = Parameters<typeof useKeyboardShortcuts>[0];

function Probe({ handlers }: { handlers: Handlers }): ReactElement {
  useKeyboardShortcuts(handlers);
  return <></>;
}

function mountHook(handlers: Handlers) {
  return render(<Probe handlers={handlers} />);
}

function dispatchKey(init: KeyboardEventInit): { prevented: boolean } {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });

  act(() => {
    window.dispatchEvent(event);
  });

  return { prevented: event.defaultPrevented };
}

describe('new-feature binding', () => {
  it.each([
    ['Cmd+T', { key: 't', metaKey: true }],
    ['Ctrl+T', { key: 't', ctrlKey: true }],
  ])('%s fires onNewFeature and suppresses the new-tab fallback', (_label, init) => {
    const onNewFeature = vi.fn();
    const onCloseCurrentView = vi.fn();
    const onOpenCommandPalette = vi.fn();

    mountHook({ onNewFeature, onCloseCurrentView, onOpenCommandPalette });

    expect(dispatchKey(init).prevented).toBe(true);
    expect(onNewFeature).toHaveBeenCalledTimes(1);
    expect(onCloseCurrentView).not.toHaveBeenCalled();
    expect(onOpenCommandPalette).not.toHaveBeenCalled();
  });
});

// Cmd/Ctrl+Shift+T stays with the webview so reopen-closed-tab keeps working.
describe('the intentionally-ignored Cmd/Ctrl+Shift+T', () => {
  it('fires nothing and does not preventDefault', () => {
    const handlers = {
      onNewFeature: vi.fn(),
      onNewProject: vi.fn(),
      onCloseCurrentView: vi.fn(),
      onNextFeature: vi.fn(),
    };

    mountHook(handlers);

    expect(dispatchKey({ key: 'T', metaKey: true, shiftKey: true }).prevented).toBe(false);

    for (const handler of Object.values(handlers)) {
      expect(handler).not.toHaveBeenCalled();
    }
  });
});

describe('F1', () => {
  it('fires onOpenDocs with no modifier held', () => {
    const onOpenDocs = vi.fn();

    mountHook({ onOpenDocs });

    expect(dispatchKey({ key: 'F1' }).prevented).toBe(true);
    expect(onOpenDocs).toHaveBeenCalledTimes(1);
  });

  // Feedback rule: the webview's own help binding must never surface, even on
  // a screen that wires no docs handler.
  it('preventDefaults unconditionally, even with no handler wired', () => {
    mountHook({});

    expect(dispatchKey({ key: 'F1' }).prevented).toBe(true);
  });
});

describe('close-view binding', () => {
  it.each([
    ['Cmd+W', { key: 'w', metaKey: true }],
    ['Ctrl+W', { key: 'w', ctrlKey: true }],
  ])('%s fires onCloseCurrentView and suppresses the window-close fallback', (_label, init) => {
    const onCloseCurrentView = vi.fn();
    const onNewFeature = vi.fn();

    mountHook({ onCloseCurrentView, onNewFeature });

    expect(dispatchKey(init).prevented).toBe(true);
    expect(onCloseCurrentView).toHaveBeenCalledTimes(1);
    expect(onNewFeature).not.toHaveBeenCalled();
  });
});
