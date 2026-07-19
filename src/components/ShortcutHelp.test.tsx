// Tests for the keyboard/mouse shortcut help overlay.
//
// The overlay self-installs its own F1 / `?` listeners rather than being wired
// from App.tsx, so most of what matters here is which keys it consumes and,
// just as importantly, which it deliberately leaves alone: Escape while closed,
// Cmd+?, and `?` typed into an input all belong to someone else.

import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { ShortcutHelp, ShortcutHelpBridge } from './ShortcutHelp';
import {
  SHORTCUTS_HELP_OPEN_EVENT,
  SHORTCUTS_HELP_CLOSE_EVENT,
  SHORTCUTS_HELP_TOGGLE_EVENT,
} from '../context/ShortcutsContext';

interface DispatchResult {
  prevented: boolean;
  stopped: boolean;
}

function dispatchKey(init: KeyboardEventInit, target: EventTarget = window): DispatchResult {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });

  let stopped = false;
  const originalStop = event.stopPropagation.bind(event);
  event.stopPropagation = () => {
    stopped = true;
    originalStop();
  };

  act(() => {
    target.dispatchEvent(event);
  });

  return { prevented: event.defaultPrevented, stopped };
}

function dispatchAppEvent(name: string): void {
  act(() => {
    window.dispatchEvent(new CustomEvent(name));
  });
}

// The overlay portals into `container`; pointing it at document.body lets the
// standard `screen` queries see it.
function mountOverlay() {
  return render(<ShortcutHelp platform="other" container={document.body} />);
}

const overlay = () => screen.queryByTestId('shortcut-help-overlay');

describe('opening the overlay', () => {
  it('starts closed', () => {
    mountOverlay();

    expect(overlay()).not.toBeInTheDocument();
  });

  it.each([
    ['F1', { key: 'F1' }],
    ['?', { key: '?' }],
  ])('opens on %s and suppresses the webview help binding', (_label, init) => {
    mountOverlay();

    expect(dispatchKey(init).prevented).toBe(true);
    expect(overlay()).toBeInTheDocument();
  });
});

describe('keys the overlay must not steal', () => {
  // These belong to the docs panel, not to help.
  it.each([
    ['Cmd+?', { key: '?', metaKey: true }],
    ['Ctrl+?', { key: '?', ctrlKey: true }],
    ['Cmd+F1', { key: 'F1', metaKey: true }],
  ])('ignores %s', (_label, init) => {
    mountOverlay();

    expect(dispatchKey(init).prevented).toBe(false);
    expect(overlay()).not.toBeInTheDocument();
  });

  it('ignores `?` typed into a focused input', async () => {
    const input = document.createElement('input');
    input.type = 'text';
    document.body.appendChild(input);
    input.focus();
    expect(document.activeElement).toBe(input);

    mountOverlay();

    const result = dispatchKey({ key: '?' }, input);

    expect(result.prevented).toBe(false);
    expect(overlay()).not.toBeInTheDocument();

    input.remove();
  });

  // App.tsx's global Escape handler drives "close modal / pop navigation"; the
  // overlay may only consume Escape while it is actually showing.
  it('leaves Escape alone while closed', () => {
    mountOverlay();

    expect(dispatchKey({ key: 'Escape' }).prevented).toBe(false);
  });
});

describe('dismissing the overlay', () => {
  it('closes on Escape, consuming the key so the global handler does not double-fire', () => {
    mountOverlay();
    dispatchKey({ key: 'F1' });
    expect(overlay()).toBeInTheDocument();

    const result = dispatchKey({ key: 'Escape' });

    expect(result.prevented).toBe(true);
    expect(result.stopped).toBe(true);
    expect(overlay()).not.toBeInTheDocument();
  });

  it.each(['shortcut-help-backdrop', 'shortcut-help-close'])(
    'closes when %s is clicked',
    async (testId) => {
      mountOverlay();
      dispatchKey({ key: 'F1' });

      await userEvent.click(screen.getByTestId(testId));

      expect(overlay()).not.toBeInTheDocument();
    },
  );

  it('stays open when the click lands inside the panel body', async () => {
    mountOverlay();
    dispatchKey({ key: 'F1' });

    await userEvent.click(screen.getByTestId('shortcut-help-body'));

    expect(overlay()).toBeInTheDocument();
  });
});

describe('the portal target', () => {
  it('anchors each instance at its own configured container', () => {
    const a = document.createElement('div');
    const b = document.createElement('div');
    document.body.append(a, b);

    render(
      <>
        <ShortcutHelp platform="other" container={a} />
        <ShortcutHelp platform="other" container={b} />
      </>,
    );

    dispatchKey({ key: 'F1' });

    // Each instance installs its own listener and owns its own portal target.
    expect(within(a).getByTestId('shortcut-help-overlay')).toBeInTheDocument();
    expect(within(b).getByTestId('shortcut-help-overlay')).toBeInTheDocument();

    a.remove();
    b.remove();
  });
});

describe('ShortcutHelpBridge', () => {
  it('self-mounts with no props and portals into document.body', () => {
    render(<ShortcutHelpBridge />);
    expect(overlay()).not.toBeInTheDocument();

    dispatchAppEvent(SHORTCUTS_HELP_OPEN_EVENT);
    expect(overlay()).toBeInTheDocument();

    dispatchAppEvent(SHORTCUTS_HELP_CLOSE_EVENT);
    expect(overlay()).not.toBeInTheDocument();
  });

  it('round-trips the toggle event', () => {
    render(<ShortcutHelpBridge />);

    dispatchAppEvent(SHORTCUTS_HELP_TOGGLE_EVENT);
    expect(overlay()).toBeInTheDocument();

    dispatchAppEvent(SHORTCUTS_HELP_TOGGLE_EVENT);
    expect(overlay()).not.toBeInTheDocument();
  });
});

describe('the rendered registry', () => {
  // Chords render through `formatEntryChords(entry, platform)`, so the glyphs
  // follow the `platform` prop rather than the universal "Cmd/Ctrl + …" form.
  it('renders non-mac chords with the Ctrl prefix', () => {
    mountOverlay();
    dispatchKey({ key: 'F1' });

    const panel = within(screen.getByTestId('shortcut-help-body'));

    // Ctrl + K appears twice (once in the Quick Reference callout at
    // the top of the body, once in the palette group's registry) —
    // both must be present.
    expect(panel.getAllByText('Ctrl + K').length).toBeGreaterThanOrEqual(2);
    // Ctrl + T opens New Feature; Ctrl + Shift + T opens the New-terminal launcher.
    expect(panel.getByText('Ctrl + T')).toBeInTheDocument();
    expect(panel.getByText('New terminal')).toBeInTheDocument();
    // "New Feature" is listed twice in the registry: Cmd/Ctrl+T and
    // its Cmd/Ctrl+Shift+N alias.
    expect(panel.getAllByText('New Feature')).toHaveLength(2);
  });

  it('renders mac chords with the ⌘ glyph', () => {
    render(<ShortcutHelp platform="mac" container={document.body} />);
    dispatchKey({ key: 'F1' });

    const panel = within(screen.getByTestId('shortcut-help-body'));

    expect(panel.getAllByText('⌘K').length).toBeGreaterThanOrEqual(2);
    expect(panel.getByText('⌘T')).toBeInTheDocument();
  });
});

describe('the Quick Reference callout (Cmd/Ctrl + `)', () => {
  // The new terminal-panel shortcut (spec §3 (f)) is documented in two
  // places: the prominent Quick Reference callout at the top of the
  // overlay, and the regular registry below it. Both must render the
  // chord for the active platform.

  it('shows the backtick shortcut alongside Cmd/Ctrl+K in the callout', () => {
    render(<ShortcutHelp platform="other" container={document.body} />);
    dispatchKey({ key: 'F1' });

    const callout = within(screen.getByTestId('shortcut-help-quick-reference'));
    // Both chords present (universal "Cmd/Ctrl + …" form).
    expect(callout.getByText('Ctrl + `')).toBeInTheDocument();
    expect(callout.getByText('Ctrl + K')).toBeInTheDocument();
  });

  it('renders the backtick shortcut in mac glyph form when the overlay is on mac', () => {
    render(<ShortcutHelp platform="mac" container={document.body} />);
    dispatchKey({ key: 'F1' });

    const callout = within(screen.getByTestId('shortcut-help-quick-reference'));
    expect(callout.getByText('⌘`')).toBeInTheDocument();
    expect(callout.getByText('⌘K')).toBeInTheDocument();
  });
});
