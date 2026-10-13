/**
 * The arithmetic is `lib/uiZoom.test.ts`; what needs a DOM is which events
 * reach it, which ones it must leave alone, and the order the stored level and
 * a keypress are allowed to arrive in.
 */

import { act, render, screen } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { UI_PREF_WRITE_DEBOUNCE_MS } from '../lib/uiPrefs';
import { UI_ZOOM_INDICATOR_MS, UiZoomIndicator } from './UiZoomIndicator';

const setZoom = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ setZoom }),
}));

function storeHolds(value: string | null) {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    Promise.resolve(cmd === 'get_app_session' ? value : undefined),
  );
}

async function mount() {
  const view = render(<UiZoomIndicator />);
  await act(async () => {});
  return view;
}

function press(init: KeyboardEventInit): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init });
  act(() => {
    document.body.dispatchEvent(event);
  });
  return event;
}

function wheel(init: WheelEventInit, target: EventTarget = document.body): WheelEvent {
  const event = new WheelEvent('wheel', { bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

beforeEach(() => {
  setZoom.mockReset();
  setZoom.mockResolvedValue(undefined);
  storeHolds(null);
});

describe('UiZoomIndicator', () => {
  it('restores the stored level on launch without announcing it', async () => {
    storeHolds('1.25');
    await mount();
    expect(setZoom).toHaveBeenCalledWith(1.25);
    expect(screen.queryByTestId('ui-zoom-indicator')).toBeNull();
  });

  it('leaves the webview alone when nothing is stored', async () => {
    await mount();
    expect(setZoom).not.toHaveBeenCalled();
  });

  it('zooms one rung on Cmd+= and names the level', async () => {
    await mount();
    const event = press({ key: '=', metaKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(setZoom).toHaveBeenCalledWith(1.1);
    expect(screen.getByTestId('ui-zoom-indicator')).toHaveTextContent('110%');
  });

  it('steps from the restored level, not from 100%', async () => {
    storeHolds('1.5');
    await mount();
    press({ key: '-', ctrlKey: true });
    expect(setZoom).toHaveBeenLastCalledWith(1.25);
  });

  it('keeps the chord from a listener further down, such as a terminal', async () => {
    await mount();
    const inner = vi.fn();
    document.body.addEventListener('keydown', inner);
    press({ key: '-', ctrlKey: true });
    document.body.removeEventListener('keydown', inner);
    expect(inner).not.toHaveBeenCalled();
  });

  it('answers a press at the end of the ladder without calling the webview', async () => {
    storeHolds('2');
    await mount();
    setZoom.mockClear();
    press({ key: '=', metaKey: true });
    expect(setZoom).not.toHaveBeenCalled();
    expect(screen.getByTestId('ui-zoom-indicator')).toHaveTextContent('200%');
  });

  it('zooms on a ctrl-wheel notch and cancels the native zoom', async () => {
    await mount();
    const event = wheel({ deltaY: -100, ctrlKey: true });
    expect(event.defaultPrevented).toBe(true);
    expect(setZoom).toHaveBeenCalledWith(1.1);
  });

  it('leaves an ordinary scroll alone', async () => {
    await mount();
    const event = wheel({ deltaY: -100 });
    expect(event.defaultPrevented).toBe(false);
    expect(setZoom).not.toHaveBeenCalled();
  });

  it('yields a ctrl-wheel a canvas has already claimed', async () => {
    const { container } = await mount();
    const pane = document.createElement('div');
    container.appendChild(pane);
    pane.addEventListener('wheel', (event) => event.preventDefault());
    wheel({ deltaY: -100, ctrlKey: true }, pane);
    expect(setZoom).not.toHaveBeenCalled();
  });

  it('hides again after a moment', async () => {
    vi.useFakeTimers();
    try {
      await mount();
      press({ key: '=', metaKey: true });
      expect(screen.getByTestId('ui-zoom-indicator')).toBeInTheDocument();
      act(() => {
        vi.advanceTimersByTime(UI_ZOOM_INDICATOR_MS);
      });
      expect(screen.queryByTestId('ui-zoom-indicator')).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('stores the level the user chose', async () => {
    vi.useFakeTimers();
    try {
      await mount();
      press({ key: '=', metaKey: true });
      await act(async () => {
        vi.advanceTimersByTime(UI_PREF_WRITE_DEBOUNCE_MS);
      });
      expect(invoke).toHaveBeenCalledWith('set_app_session', { key: 'ui.zoom', value: '1.1' });
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps a keypress that beat the stored level, and stores it', async () => {
    vi.useFakeTimers();
    try {
      let answer: (value: string) => void = () => {};
      vi.mocked(invoke).mockImplementation((cmd: string) =>
        cmd === 'get_app_session'
          ? new Promise<string>((resolve) => {
              answer = resolve;
            })
          : Promise.resolve(undefined),
      );
      render(<UiZoomIndicator />);
      press({ key: '=', metaKey: true });
      await act(async () => {
        answer('1.75');
      });
      expect(setZoom).toHaveBeenCalledTimes(1);
      expect(setZoom).toHaveBeenLastCalledWith(1.1);
      await act(async () => {
        vi.advanceTimersByTime(UI_PREF_WRITE_DEBOUNCE_MS);
      });
      expect(invoke).toHaveBeenCalledWith('set_app_session', { key: 'ui.zoom', value: '1.1' });
    } finally {
      vi.useRealTimers();
    }
  });
});
