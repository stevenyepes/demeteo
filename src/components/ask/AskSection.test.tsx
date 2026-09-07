/**
 * Project Home's Ask tab: that a project's threads are *listed*, and that the
 * filter above them is the session bar the Discovery tab uses.
 *
 * The list is what this tab did not have — the tab was a launcher into
 * whichever thread the workspace seeded, and the only enumeration lived in a
 * dropdown behind that workspace. So the assertions here are about the set of
 * cards on screen, never about which one opens.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AskSection } from './AskSection';
import { listAskThreads } from '../../lib/ask';
import type { AskThread } from '../../types';

vi.mock('../../lib/ask', () => ({
  listAskThreads: vi.fn(),
  EVENT_ASK_TURN_STATUS: 'ask_turn_status',
}));

vi.mock('./NewAskThreadModal', () => ({
  NewAskThreadModal: ({ seedTitle }: { seedTitle: string }) => (
    <div data-testid="new-ask-thread-modal">{seedTitle}</div>
  ),
}));

const listAskThreadsMock = vi.mocked(listAskThreads);

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
    network: false,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

function renderSection(onOpen = vi.fn()) {
  render(<AskSection projectId="p1" machineId="local" projectName="demeteo" onOpen={onOpen} />);
  return { onOpen };
}

beforeEach(() => {
  listAskThreadsMock.mockReset();
});

describe('AskSection', () => {
  it('lists every thread in the project, open and closed', async () => {
    listAskThreadsMock.mockResolvedValue([
      thread({ id: 't1', title: 'user journey' }),
      thread({ id: 't2', title: 'keepalive', status: 'closed' }),
    ]);

    renderSection();

    expect(await screen.findByText('user journey')).toBeInTheDocument();
    expect(screen.getByText('keepalive')).toBeInTheDocument();
    expect(screen.getAllByTestId('ask-thread-card')).toHaveLength(2);
  });

  it('opens the thread a card names rather than whichever one seeds first', async () => {
    listAskThreadsMock.mockResolvedValue([
      thread({ id: 't1', title: 'user journey' }),
      thread({ id: 't2', title: 'keepalive' }),
    ]);

    const { onOpen } = renderSection();

    fireEvent.click(await screen.findByText('keepalive'));
    expect(onOpen).toHaveBeenCalledWith('t2');
  });

  it('narrows the list by status through the shared session bar', async () => {
    listAskThreadsMock.mockResolvedValue([
      thread({ id: 't1', title: 'user journey' }),
      thread({ id: 't2', title: 'keepalive', status: 'closed' }),
    ]);

    renderSection();

    await screen.findByTestId('ask-filter-bar');
    fireEvent.click(screen.getByRole('radio', { name: /Closed/ }));

    await waitFor(() => expect(screen.getAllByTestId('ask-thread-card')).toHaveLength(1));
    expect(screen.getByText('keepalive')).toBeInTheDocument();
    expect(screen.queryByText('user journey')).toBeNull();
  });

  it('narrows the list by title text', async () => {
    listAskThreadsMock.mockResolvedValue([
      thread({ id: 't1', title: 'user journey' }),
      thread({ id: 't2', title: 'keepalive' }),
    ]);

    renderSection();

    fireEvent.change(await screen.findByLabelText('Filter threads by text'), {
      target: { value: 'keep' },
    });

    await waitFor(() => expect(screen.getAllByTestId('ask-thread-card')).toHaveLength(1));
    expect(screen.getByText('keepalive')).toBeInTheDocument();
  });

  it('offers the agent filter only once a project has run more than one harness', async () => {
    listAskThreadsMock.mockResolvedValue([thread({ id: 't1' }), thread({ id: 't2' })]);
    renderSection();
    await screen.findByTestId('ask-filter-bar');
    expect(screen.queryByLabelText('Filter by agent')).toBeNull();

    cleanup();
    listAskThreadsMock.mockResolvedValue([
      thread({ id: 't1', title: 'a', agent_kind: 'codex' }),
      thread({ id: 't2', title: 'b', agent_kind: 'claude-code' }),
    ]);
    renderSection();

    const agent = await screen.findByLabelText('Filter by agent');
    fireEvent.change(agent, { target: { value: 'codex' } });

    await waitFor(() => expect(screen.getAllByTestId('ask-thread-card')).toHaveLength(1));
    expect(screen.getByText('a')).toBeInTheDocument();
  });

  it('renders the empty state, and no filter bar, for a project with no threads', async () => {
    listAskThreadsMock.mockResolvedValue([]);

    renderSection();

    expect(await screen.findByText('No threads yet')).toBeInTheDocument();
    expect(screen.queryByTestId('ask-filter-bar')).toBeNull();
  });

  it('reports a failed list without hiding the composer', async () => {
    listAskThreadsMock.mockRejectedValue(new Error('nope'));

    renderSection();

    expect(await screen.findByRole('alert')).toHaveTextContent('nope');
    expect(screen.getByTestId('ask-composer')).toBeInTheDocument();
  });

  it('carries the composer text into the new-thread modal', async () => {
    listAskThreadsMock.mockResolvedValue([]);

    renderSection();

    fireEvent.change(await screen.findByTestId('ask-composer'), {
      target: { value: 'why does sync revert my branch' },
    });
    fireEvent.click(screen.getByTestId('open-new-ask-thread'));

    expect(screen.getByTestId('new-ask-thread-modal')).toHaveTextContent(
      'why does sync revert my branch',
    );
  });
});
