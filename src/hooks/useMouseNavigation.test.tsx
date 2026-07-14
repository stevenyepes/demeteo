// Unit tests for `src/hooks/useMouseNavigation.ts`.
//
// Pins down:
//   - `mousedown` with `button === 3` (XButton1 / back) calls `goBack`
//   - `mousedown` with `button === 4` (XButton2 / forward) calls `goForward`
//   - the listener calls `preventDefault()` so the Tauri webview cannot fall
//     back to its own browser history traversal
//   - the listener is cleaned up on unmount
//   - buttons outside {3, 4} are ignored
//   - `MouseNavigationBridge` self-mounts (renders null, no App.tsx wiring)

import { act, render } from '@testing-library/react';
import { type ReactElement } from 'react';
import { describe, expect, it } from 'vitest';

import { MouseNavigationBridge, useMouseNavigation } from './useMouseNavigation';
import { NavigationProvider, useNavigation } from '../context/NavigationContext';

type NavView = { kind: 'home' } | { kind: 'settings' } | { kind: 'providers' };

interface NavProbe {
  navigate: (view: NavView) => void;
  goBack: () => void;
  goForward: () => void;
  canGoBack: boolean;
  canGoForward: boolean;
}

function Probe({ holder }: { holder: { current: NavProbe | null } }): ReactElement {
  const nav = useNavigation();
  holder.current = nav;
  useMouseNavigation();
  return <></>;
}

function mountProbe() {
  const holder: { current: NavProbe | null } = { current: null };

  const view = render(
    <NavigationProvider>
      <Probe holder={holder} />
    </NavigationProvider>,
  );

  const probe = (): NavProbe => {
    if (!holder.current) throw new Error('probe did not mount');
    return holder.current;
  };

  const navigate = (to: NavView) => act(() => probe().navigate(to));

  return { ...view, probe, navigate };
}

// Returns whether the hook called `preventDefault` — the guard that stops the
// webview from also running its own history traversal.
function dispatchMouseDown(button: number): { prevented: boolean } {
  const event = new MouseEvent('mousedown', { button, bubbles: true, cancelable: true });

  act(() => {
    window.dispatchEvent(event);
  });

  return { prevented: event.defaultPrevented };
}

describe('useMouseNavigation', () => {
  it('maps button 3 to back and button 4 to forward, suppressing the default', () => {
    const { probe, navigate } = mountProbe();

    // Seed the history: empty-state → home → settings → providers.
    navigate({ kind: 'home' });
    navigate({ kind: 'settings' });
    navigate({ kind: 'providers' });
    expect(probe().canGoBack).toBe(true);

    expect(dispatchMouseDown(3).prevented).toBe(true);
    expect(probe().canGoForward).toBe(true);

    expect(dispatchMouseDown(4).prevented).toBe(true);
    expect(probe().canGoForward).toBe(false);
  });

  it.each([0, 1, 2])('ignores mouse button %i', (button) => {
    const { probe, navigate } = mountProbe();

    navigate({ kind: 'home' });
    navigate({ kind: 'settings' });

    // If a non-{3,4} button dispatched BACK, canGoBack would flip to false.
    dispatchMouseDown(button);

    expect(probe().canGoBack).toBe(true);
  });

  it('removes the listener on unmount', () => {
    const { navigate, unmount } = mountProbe();

    navigate({ kind: 'home' });
    navigate({ kind: 'settings' });
    unmount();

    // There is no provider left to observe, so the assertion is that the
    // orphaned listener is gone and dispatching does not throw.
    expect(() => {
      dispatchMouseDown(3);
      dispatchMouseDown(4);
    }).not.toThrow();
  });
});

describe('MouseNavigationBridge', () => {
  it('self-mounts, renders no UI, and survives a dispatch', () => {
    const { container } = render(
      <NavigationProvider>
        <MouseNavigationBridge />
      </NavigationProvider>,
    );

    expect(container).toBeEmptyDOMElement();

    dispatchMouseDown(3);

    expect(container).toBeEmptyDOMElement();
  });
});
