import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { listen } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AskWorkspaceHeader } from './AskWorkspaceHeader';
import { listAskThreads } from '../../lib/ask';
import type { AskThread } from '../../types';

vi.mock('../../lib/ask', () => ({
  listAskThreads: vi.fn(),
  EVENT_ASK_TURN_STATUS: 'ask_turn_status',
}));

afterEach(cleanup);

function metric(label: string): HTMLElement {
  const found = screen.getByTestId('metric-strip').querySelector(`[data-metric="${label}"]`);
  if (found === null) throw new Error(`no metric ${label}`);
  return found as HTMLElement;
}

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
    cost_usd: 0.42,
    tokens: 48200,
    network: true,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

beforeEach(() => {
  vi.mocked(listAskThreads).mockResolvedValue([]);
  vi.mocked(listen).mockImplementation(async () => () => {});
});

describe('AskWorkspaceHeader', () => {
  it("renders title, kind chip, and Turns/Spend/Tokens straight off the thread row", () => {
    render(
      <AskWorkspaceHeader
        thread={thread()}
        projectId="p1"
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onOpenSettings={vi.fn()}
      />,
    );

    expect(screen.getByText('How a Step reaches the feature branch')).toBeInTheDocument();
    expect(screen.getByText('claude-code')).toBeInTheDocument();

    expect(metric('Turns')).toHaveTextContent('4');
    expect(metric('Spend')).toHaveTextContent('$0.420');
    expect(metric('Tokens')).toHaveTextContent('48.2k');
  });

  it('opens AskThreadSwitcher from the Threads trigger', async () => {
    vi.mocked(listAskThreads).mockResolvedValue([thread()]);

    render(
      <AskWorkspaceHeader
        thread={thread()}
        projectId="p1"
        onSelectThread={vi.fn()}
        onNewThread={vi.fn()}
        onOpenSettings={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByTestId('ask-thread-switcher-trigger'));

    expect(await screen.findByTestId('ask-thread-switcher-menu')).toBeInTheDocument();
  });

  it('calls onNewThread when New thread is clicked', () => {
    const onNewThread = vi.fn();
    render(
      <AskWorkspaceHeader
        thread={thread()}
        projectId="p1"
        onSelectThread={vi.fn()}
        onNewThread={onNewThread}
        onOpenSettings={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByTestId('ask-new-thread'));

    expect(onNewThread).toHaveBeenCalledTimes(1);
  });
});
