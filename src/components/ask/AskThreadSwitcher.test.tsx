import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { listen, type EventCallback } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AskThreadSwitcher } from './AskThreadSwitcher';
import { listAskThreads } from '../../lib/ask';
import type { AskThread } from '../../types';

vi.mock('../../lib/ask', () => ({
  listAskThreads: vi.fn(),
  EVENT_ASK_TURN_STATUS: 'ask_turn_status',
}));

afterEach(cleanup);

function thread(overrides: Partial<AskThread> = {}): AskThread {
  return {
    id: 't1',
    project_id: 'p1',
    title: 'How a Step reaches the feature branch',
    status: 'open',
    agent_kind: 'claude-code',
    model: null,
    effort: null,
    machine_id: 'local',
    worktree_path: null,
    session_id: null,
    turn_count: 4,
    cost_usd: 0,
    tokens: 0,
    network: true,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

/** The handler the switcher registered for `ask_turn_status`. */
let statusHandler: EventCallback<unknown> | null = null;

beforeEach(() => {
  statusHandler = null;
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    if (event === 'ask_turn_status') statusHandler = handler;
    return () => {};
  });
});

describe('AskThreadSwitcher', () => {
  it('lists title, kind chip and turn count per thread, sourced from listAskThreads', async () => {
    vi.mocked(listAskThreads).mockResolvedValue([
      thread({ id: 't1', title: 'How a Step reaches the feature branch', agent_kind: 'claude-code', turn_count: 4 }),
    ]);

    render(<AskThreadSwitcher projectId="p1" activeThreadId={null} onSelect={vi.fn()} />);

    fireEvent.click(await screen.findByTestId('ask-thread-switcher-trigger'));

    const row = await screen.findByTestId('ask-thread-switcher-row');
    expect(row).toHaveTextContent('How a Step reaches the feature branch');
    expect(row).toHaveTextContent('claude-code');
    expect(row).toHaveTextContent('4 turns');
  });

  it('calls onSelect with the thread id and does not navigate itself', async () => {
    vi.mocked(listAskThreads).mockResolvedValue([thread({ id: 't1' })]);
    const onSelect = vi.fn();

    render(<AskThreadSwitcher projectId="p1" activeThreadId={null} onSelect={onSelect} />);
    fireEvent.click(await screen.findByTestId('ask-thread-switcher-trigger'));
    fireEvent.click(await screen.findByTestId('ask-thread-switcher-row'));

    expect(onSelect).toHaveBeenCalledWith('t1');
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('marks a thread live from ask_turn_status, not from a stored column', async () => {
    vi.mocked(listAskThreads).mockResolvedValue([thread({ id: 't1' })]);

    render(<AskThreadSwitcher projectId="p1" activeThreadId={null} onSelect={vi.fn()} />);
    await waitFor(() => expect(statusHandler).not.toBeNull());

    fireEvent.click(await screen.findByTestId('ask-thread-switcher-trigger'));
    await screen.findByTestId('ask-thread-switcher-row');
    expect(screen.queryByText('live')).not.toBeInTheDocument();

    statusHandler!({
      event: 'ask_turn_status',
      id: 1,
      payload: { thread_id: 't1', status: 'running', reason: null },
    });
    expect(await screen.findByText('live')).toBeInTheDocument();

    statusHandler!({
      event: 'ask_turn_status',
      id: 2,
      payload: { thread_id: 't1', status: 'idle', reason: null },
    });
    await waitFor(() => expect(screen.queryByText('live')).not.toBeInTheDocument());
  });

  it('highlights the active thread’s row', async () => {
    vi.mocked(listAskThreads).mockResolvedValue([thread({ id: 't1' }), thread({ id: 't2', title: 'Other thread' })]);

    render(<AskThreadSwitcher projectId="p1" activeThreadId="t2" onSelect={vi.fn()} />);
    fireEvent.click(await screen.findByTestId('ask-thread-switcher-trigger'));

    const rows = await screen.findAllByTestId('ask-thread-switcher-row');
    expect(rows[0]).toHaveAttribute('data-active', 'false');
    expect(rows[1]).toHaveAttribute('data-active', 'true');
  });
});
