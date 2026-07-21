import { render } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TerminalApprovalRecognizer } from './TerminalApprovalRecognizer';
import type { TerminalTabDescriptor } from '../types';

// A controllable headless xterm: `write(bytes)` replaces the visible buffer with
// the decoded text, bottom-aligned into `rows` so `readBottomRows` sees it.
vi.mock('@xterm/headless', () => {
  class HeadlessStub {
    rows = 24;
    private lines: string[] = new Array(24).fill('');
    buffer = {
      active: {
        baseY: 0,
        getLine: (y: number) => {
          const s = this.lines[y];
          return s === undefined ? undefined : { translateToString: () => s };
        },
      },
    };
    write(data: Uint8Array | string) {
      const text =
        typeof data === 'string' ? data : new TextDecoder().decode(data);
      const content = text.split('\n');
      const pad = new Array(Math.max(0, this.rows - content.length)).fill('');
      this.lines = pad.concat(content).slice(-this.rows);
    }
    dispose = vi.fn();
  }
  return { Terminal: HeadlessStub };
});

type Chan = { id: number; onmessage: ((m: Uint8Array | number[]) => void) | null };

/** Per-test capture: the channel the recognizer attached, and the
 *  `report_terminal_screen_activity` calls it made. */
let attachedChannel: Chan | null;
let reports: Array<{ sessionId: string; present: boolean }>;
let detaches: number[];

function agentTab(overrides: Partial<TerminalTabDescriptor> = {}): TerminalTabDescriptor {
  return {
    tabId: 'tab-1',
    sessionId: 'sess-1',
    machineId: 'local',
    machineLabel: 'local',
    title: 'Codex',
    phase: 'running',
    createdAt: 0,
    agentKind: 'codex',
    ...overrides,
  };
}

beforeEach(() => {
  attachedChannel = null;
  reports = [];
  detaches = [];
  vi.useFakeTimers();
  vi.mocked(invoke).mockImplementation(((cmd: string, rawArgs?: unknown) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case 'attach_terminal_session':
        attachedChannel = args?.tauriChannel as Chan;
        return Promise.resolve(undefined);
      case 'detach_terminal_session':
        detaches.push((args?.channelId as number) ?? -1);
        return Promise.resolve(undefined);
      case 'report_terminal_screen_activity':
        reports.push({
          sessionId: args?.sessionId as string,
          present: args?.present as boolean,
        });
        return Promise.resolve(undefined);
      default:
        return Promise.resolve(undefined);
    }
  }) as typeof invoke);
});

afterEach(() => {
  vi.useRealTimers();
});

/** Feed a chunk into the recognizer's headless buffer and let the debounced
 *  scan loop settle (two confirmations, 150ms apart, plus slack). */
async function feed(text: string) {
  attachedChannel?.onmessage?.(new TextEncoder().encode(text));
  await vi.advanceTimersByTimeAsync(400);
}

describe('TerminalApprovalRecognizer', () => {
  it('attaches a headless buffer for a packed agent session', () => {
    render(<TerminalApprovalRecognizer tabs={[agentTab()]} />);
    expect(attachedChannel).not.toBeNull();
  });

  it('does NOT recognize plain shells or hooked (Claude) agents', () => {
    render(
      <TerminalApprovalRecognizer
        tabs={[
          agentTab({ tabId: 'a', sessionId: 's-shell', agentKind: null }),
          agentTab({ tabId: 'b', sessionId: 's-claude', agentKind: 'claude-code' }),
        ]}
      />,
    );
    expect(attachedChannel).toBeNull();
  });

  it('does not attach a connecting tab with no live session', () => {
    render(
      <TerminalApprovalRecognizer
        tabs={[agentTab({ sessionId: null, phase: 'connecting' })]}
      />,
    );
    expect(attachedChannel).toBeNull();
  });

  it('reports an approval when the prompt is rendered, and retracts when it clears', async () => {
    render(<TerminalApprovalRecognizer tabs={[agentTab()]} />);
    expect(attachedChannel).not.toBeNull();

    await feed('Allow the agent to run this command? [y/n]');
    expect(reports).toContainEqual({ sessionId: 'sess-1', present: true });

    reports = [];
    await feed('Running tests...\nAll green.');
    expect(reports).toContainEqual({ sessionId: 'sess-1', present: false });
  });

  it('absorbs a single transient prompt frame (no flap)', async () => {
    render(<TerminalApprovalRecognizer tabs={[agentTab()]} />);
    // One chunk shows the prompt, but the very next overwrites it before the
    // debounce can confirm — must NOT report an approval.
    attachedChannel?.onmessage?.(
      new TextEncoder().encode('Allow this? [y/n]'),
    );
    await vi.advanceTimersByTimeAsync(150); // one scan only
    attachedChannel?.onmessage?.(new TextEncoder().encode('done'));
    await vi.advanceTimersByTimeAsync(400);
    expect(reports.some((r) => r.present)).toBe(false);
  });

  it('detaches and retracts when the session goes away', async () => {
    const { rerender } = render(<TerminalApprovalRecognizer tabs={[agentTab()]} />);
    await feed('Allow the agent to execute? [y/n]');
    expect(reports).toContainEqual({ sessionId: 'sess-1', present: true });

    reports = [];
    rerender(<TerminalApprovalRecognizer tabs={[]} />);
    expect(detaches.length).toBeGreaterThan(0);
    // The latched approval is retracted so the backend drops the stale mark.
    expect(reports).toContainEqual({ sessionId: 'sess-1', present: false });
  });
});
