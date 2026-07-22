import { act, render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { TerminalSurface } from './TerminalSurface';
import { terminalStubs, webglAddonStubs } from '../test/setup';

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
