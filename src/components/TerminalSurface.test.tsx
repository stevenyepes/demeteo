import { act, render, type RenderResult } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { describe, expect, it, vi } from 'vitest';

import { TerminalSurface, type TerminalSurfaceProps } from './TerminalSurface';
import { getLastTerminalSize, setLastTerminalSize } from '../lib/terminalViewport';
import {
  DEFAULT_FIT_COLS,
  DEFAULT_FIT_ROWS,
  fitAddonStubs,
  resizeObserverStubs,
  setFitGeometry,
  terminalStubs,
  webglAddonStubs,
} from '../test/setup';

function surface(overrides: Partial<TerminalSurfaceProps> = {}): React.ReactElement {
  return (
    <TerminalSurface
      tabId="tab-1"
      sessionId="sess-1"
      phase="running"
      title="repo"
      machineLabel="local"
      machineId="local"
      {...overrides}
    />
  );
}

function renderSurface(overrides: Partial<TerminalSurfaceProps> = {}) {
  return render(surface(overrides));
}

// jsdom computes no layout, so every element answers the boxless test that
// `hasLayoutBox` is built on. Stubbing the border box is the only way to say
// "this surface is on screen" — `offsetParent` is not writable per-element and
// is null under jsdom regardless of `display`.
function giveContainerLayoutBox(view: RenderResult): void {
  const el = view.container.querySelector<HTMLElement>('.w-full.h-full');
  if (!el) throw new Error('terminal container not found');
  el.getBoundingClientRect = () =>
    ({
      width: 800,
      height: 400,
      top: 0,
      left: 0,
      right: 800,
      bottom: 400,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    }) as DOMRect;
}

function resizeInvokes(): unknown[][] {
  return vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'resize_terminal_session');
}

describe('TerminalSurface WebGL context loss', () => {
  it('falls back to the DOM renderer and repaints the current viewport', async () => {
    render(
      <TerminalSurface
        tabId="tab-1"
        sessionId="sess-1"
        phase="running"
        title="repo"
        machineLabel="local"
        machineId="local"
      />,
    );

    // The mount effect creates one Terminal + one WebglAddon instance.
    const webgl = webglAddonStubs[0];
    const term = terminalStubs[0];
    expect(webgl).toBeDefined();
    expect(webgl.contextLossHandler).toBeInstanceOf(Function);

    await act(async () => {
      webgl.contextLossHandler?.();
    });

    // The addon disposes itself so xterm hands rendering back to the DOM
    // renderer, and the surface forces a repaint of the current viewport so
    // stale rows don't stay blank until unrelated output touches them.
    expect(webgl.dispose).toHaveBeenCalledTimes(1);
    expect(term.refresh).toHaveBeenCalledTimes(1);
    expect(term.refresh).toHaveBeenCalledWith(0, term.rows - 1);
  });
});

describe('TerminalSurface geometry', () => {
  it('does not resize or cache when the container has no layout box', async () => {
    setLastTerminalSize(120, 40);
    const cachedBefore = getLastTerminalSize();
    const persistedBefore = localStorage.getItem('demeteo.terminal.lastSize');
    // A geometry that would pass every downstream floor, so what this asserts is
    // the boxless guard itself and not `isPlausibleTerminalSize` standing in for
    // it. jsdom lays nothing out, so the container reports the null
    // `offsetParent` and 0 × 0 rect of a `display:none` subtree.
    setFitGeometry(100, 30);

    await act(async () => {
      renderSurface();
    });

    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });

    expect(fitAddonStubs[0]?.fit).not.toHaveBeenCalled();
    expect(resizeInvokes()).toHaveLength(0);
    expect(getLastTerminalSize()).toEqual(cachedBefore);
    expect(localStorage.getItem('demeteo.terminal.lastSize')).toBe(persistedBefore);
  });

  it('leaves the buffer alone when the proposed fit is implausible', async () => {
    setLastTerminalSize(120, 40);
    // The 11 × 5 a boxless surface proposes, reached here with a layout box so
    // the boxless guard is not what refuses it. Asserting on the *buffer*, not
    // on the PTY message: `fit()` reflows the terminal itself, so a size caught
    // only after the fit has already been applied to what the user is looking at.
    setFitGeometry(11, 5);

    const view = await act(async () => renderSurface({ visible: true }));
    giveContainerLayoutBox(view);

    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });

    const term = terminalStubs[0];
    expect(fitAddonStubs[0]?.fit).not.toHaveBeenCalled();
    expect(term.cols).toBe(DEFAULT_FIT_COLS);
    expect(term.rows).toBe(DEFAULT_FIT_ROWS);
    expect(resizeInvokes()).toHaveLength(0);
  });

  it('retries a geometry the backend refused', async () => {
    setLastTerminalSize(100, 30);
    setFitGeometry(100, 30);

    const view = await act(async () => renderSurface({ visible: true }));
    giveContainerLayoutBox(view);

    // `checked_pty_size` refuses a geometry outside the backend's bounds. The
    // surface must not remember a refused size as delivered: the equality skip
    // would then swallow every later fit that lands on it again.
    vi.mocked(invoke).mockImplementationOnce((cmd: string) =>
      cmd === 'resize_terminal_session'
        ? Promise.reject(new Error('Refused: implausible terminal size'))
        : Promise.reject(new Error(`unexpected command: ${cmd}`)),
    );

    setFitGeometry(132, 44);
    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });
    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });

    expect(resizeInvokes()).toEqual([
      ['resize_terminal_session', { sessionId: 'sess-1', cols: 132, rows: 44 }],
      ['resize_terminal_session', { sessionId: 'sess-1', cols: 132, rows: 44 }],
    ]);
  });
});

describe('TerminalSurface reconnect', () => {
  it('pushes the geometry back onto the rebuilt PTY', async () => {
    // Steady state before the drop: the session was spawned at the size this
    // surface fits to, so nothing has ever been sent.
    setLastTerminalSize(120, 40);
    setFitGeometry(120, 40);

    const view = await act(async () => renderSurface({ phase: 'disconnected', visible: true }));
    giveContainerLayoutBox(view);

    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });
    expect(resizeInvokes()).toHaveLength(0);

    // `reconnect_terminal_session` built a fresh PTY at the backend's 80 × 24
    // fallback and reported the session running again. Same surface, same
    // session id, nothing remounts.
    await act(async () => {
      view.rerender(surface({ phase: 'running', visible: true }));
    });

    expect(resizeInvokes()).toEqual([
      ['resize_terminal_session', { sessionId: 'sess-1', cols: 120, rows: 40 }],
    ]);
  });
});

describe('TerminalSurface visibility', () => {
  it('re-fits and repaints when the surface becomes visible', async () => {
    setLastTerminalSize(80, 24);
    setFitGeometry(100, 30);

    const view = await act(async () => renderSurface({ visible: false }));
    const term = terminalStubs[0];
    const fit = fitAddonStubs[0];
    expect(fit.fit).not.toHaveBeenCalled();

    giveContainerLayoutBox(view);

    await act(async () => {
      view.rerender(surface({ visible: true }));
    });

    expect(fit.fit).toHaveBeenCalled();
    expect(term.refresh).toHaveBeenCalledWith(0, term.rows - 1);
  });

  it('sends no PTY resize when the fitted size is unchanged', async () => {
    // The size the session was spawned at, and the size this surface fits to:
    // the P10k startup contract is that the round trip emits no `SIGWINCH`.
    setLastTerminalSize(100, 30);
    setFitGeometry(100, 30);

    const view = await act(async () => renderSurface({ visible: true }));
    giveContainerLayoutBox(view);

    await act(async () => {
      view.rerender(surface({ visible: false }));
    });
    await act(async () => {
      view.rerender(surface({ visible: true }));
    });

    expect(fitAddonStubs[0]?.fit).toHaveBeenCalled();
    expect(resizeInvokes()).toHaveLength(0);
  });

  it('fits on its first visible frame when it mounts hidden', async () => {
    setLastTerminalSize(80, 24);
    setFitGeometry(140, 42);

    const view = await act(async () => renderSurface({ visible: false }));
    const term = terminalStubs[0];
    expect(term.cols).toBe(DEFAULT_FIT_COLS);

    giveContainerLayoutBox(view);

    await act(async () => {
      view.rerender(surface({ visible: true }));
    });

    expect(term.cols).toBe(140);
    expect(term.rows).toBe(42);
  });

  it('emits exactly one PTY resize when the geometry changes while visible', async () => {
    setLastTerminalSize(100, 30);
    setFitGeometry(100, 30);

    const view = await act(async () => renderSurface({ visible: true }));
    giveContainerLayoutBox(view);

    setFitGeometry(132, 44);
    await act(async () => {
      resizeObserverStubs[0]?.trigger();
    });

    expect(resizeInvokes()).toEqual([
      ['resize_terminal_session', { sessionId: 'sess-1', cols: 132, rows: 44 }],
    ]);
  });
});
