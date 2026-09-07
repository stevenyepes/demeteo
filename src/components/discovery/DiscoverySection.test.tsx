/**
 * The Discovery tab's filter — the same bar and the same policy the Ask tab
 * renders (`components/SessionFilterBar.tsx`, `lib/sessionFilter.ts`), so what
 * is asserted here is the *binding*: that the tab hands the bar its whole
 * unfiltered list and renders what came back.
 *
 * Counts come off the unfiltered list on purpose. A badge that shrank as the
 * query narrowed could not tell you what the other segments hold, which is the
 * one thing a count is for.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DiscoverySection } from './DiscoverySection';
import { getDiscoveryBoard } from '../../lib/discovery';
import type { DiscoverySummary } from '../../types';

vi.mock('../../lib/discovery', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/discovery')>()),
  getDiscoveryBoard: vi.fn(),
}));

vi.mock('./NewDiscoveryModal', () => ({
  NewDiscoveryModal: () => <div data-testid="new-discovery-modal" />,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

const getDiscoveryBoardMock = vi.mocked(getDiscoveryBoard);

afterEach(cleanup);

function discovery(overrides: Partial<DiscoverySummary> = {}): DiscoverySummary {
  return {
    id: 'd1',
    project_id: 'p1',
    title: 'multi-client runner',
    status: 'open',
    machine_id: 'local',
    agent_kind: 'claude-code',
    model: null,
    effort: null,
    resume_session_id: null,
    worktree_path: null,
    attachments: [],
    total_cost: 0,
    tokens: 0,
    created_at: 0,
    updated_at: 0,
    message_count: 2,
    progress: { landed: 0, live: 0, dropped: 0, in_flight: 0, blocked: 0, ready: 0 },
    ...overrides,
  };
}

function renderSection(discoveries: DiscoverySummary[]) {
  render(
    <DiscoverySection
      projectId="p1"
      machineId="local"
      discoveries={discoveries}
      isLoading={false}
      onCreated={vi.fn()}
      onOpen={vi.fn()}
    />,
  );
}

beforeEach(() => {
  getDiscoveryBoardMock.mockReset();
  getDiscoveryBoardMock.mockRejectedValue(new Error('no board in this test'));
});

describe('DiscoverySection filtering', () => {
  it('narrows the list by status', async () => {
    renderSection([
      discovery({ id: 'd1', title: 'multi-client runner' }),
      discovery({ id: 'd2', title: 'windows parity', status: 'closed' }),
    ]);

    fireEvent.click(screen.getByRole('radio', { name: /Closed/ }));

    await waitFor(() => expect(screen.getAllByTestId('discovery-card')).toHaveLength(1));
    expect(screen.getByText('windows parity')).toBeInTheDocument();
  });

  it('narrows the list by title text', async () => {
    renderSection([
      discovery({ id: 'd1', title: 'multi-client runner' }),
      discovery({ id: 'd2', title: 'windows parity' }),
    ]);

    fireEvent.change(screen.getByLabelText('Filter discoveries by text'), {
      target: { value: 'windows' },
    });

    await waitFor(() => expect(screen.getAllByTestId('discovery-card')).toHaveLength(1));
    expect(screen.getByText('windows parity')).toBeInTheDocument();
  });

  it('counts segments over the unfiltered list, so a query cannot shrink a badge', async () => {
    renderSection([
      discovery({ id: 'd1', title: 'multi-client runner' }),
      discovery({ id: 'd2', title: 'windows parity' }),
      discovery({ id: 'd3', title: 'ssh conformance', status: 'closed' }),
    ]);

    expect(screen.getByRole('radio', { name: 'Open, 2' })).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('Filter discoveries by text'), {
      target: { value: 'windows' },
    });

    await waitFor(() => expect(screen.getAllByTestId('discovery-card')).toHaveLength(1));
    expect(screen.getByRole('radio', { name: 'Open, 2' })).toBeInTheDocument();
  });

  it('offers a reset when the filter matched nothing', async () => {
    renderSection([discovery({ id: 'd1', title: 'multi-client runner' })]);

    fireEvent.change(screen.getByLabelText('Filter discoveries by text'), {
      target: { value: 'nothing matches this' },
    });

    fireEvent.click(await screen.findByRole('button', { name: 'Clear filters' }));

    await waitFor(() => expect(screen.getAllByTestId('discovery-card')).toHaveLength(1));
  });

  it('renders no filter bar for a project with no discoveries', () => {
    renderSection([]);

    expect(screen.getByText('No discoveries yet')).toBeInTheDocument();
    expect(screen.queryByTestId('discovery-filter-bar')).toBeNull();
  });
});
