/**
 * `ActivityPanel` (UI_REDESIGN_PLAN §1 **D**, Phase 5): the run's one activity
 * surface. What these pin is the wiring the plan asked for and the two claims a
 * reader of the component alone could not check —
 *
 *  - the panel renders the feed it is *given* and accumulates none of its own,
 *    which is what stops the detached tail from being a second unbounded copy
 *    of the rows `useRemoteRun` already caps;
 *  - closing it stops the tunnel poll, and the affordance says so. Both halves
 *    matter: the saving is the point, and a feed that silently stops advancing
 *    is indistinguishable from a run that stopped.
 *
 * The run-event rows themselves are `RunEventFeed.test.tsx`'s.
 */
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const streamRemoteEvents = vi.fn();
vi.mock('../../lib/remoteRuns', () => ({
  streamRemoteEvents: (...args: unknown[]) => streamRemoteEvents(...args),
}));

import { ActivityPanel, REMOTE_TAIL_POLL_MS } from './ActivityPanel';
import type { RemoteRunMirror, RunEvent } from '../../types';

const RUN: RemoteRunMirror = {
  machine_id: 'm-1',
  run_id: 'r-1',
  project_id: 'p-1',
  title: 'Add a thing',
  status: 'running',
  error: null,
  feature_id: 'f-1',
  pr_url: null,
  pushed_branch: null,
  last_offset: 0,
  created_at: 0,
  updated_at: 0,
  last_notified_status: null,
};

const event = (over: Partial<RunEvent> = {}): RunEvent => ({
  offset: 1,
  run_id: 'r-1',
  kind: 'pr_opened',
  payload_json: JSON.stringify('https://ex/pr/1'),
  created_at: 0,
  ...over,
});

function mountRemote(
  props: Partial<React.ComponentProps<typeof ActivityPanel>> = {},
  onEvents = vi.fn(),
) {
  const result = render(
    <ActivityPanel
      events={[]}
      remote={{ run: RUN, machineName: 'gpu-box', onEvents }}
      terminal={false}
      open
      onOpenChange={() => {}}
      {...props}
    />,
  );
  return { ...result, onEvents };
}

beforeEach(() => {
  streamRemoteEvents.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  streamRemoteEvents.mockReset();
  vi.useRealTimers();
});

describe('ActivityPanel — the feed', () => {
  it('renders the events it is handed', async () => {
    mountRemote({ events: [event()] });

    expect(await screen.findByText('PR opened')).toBeInTheDocument();
  });

  it('hands fetched rows up rather than keeping a copy of its own', async () => {
    const fresh = [event({ offset: 7 })];
    streamRemoteEvents.mockResolvedValue(fresh);

    const { onEvents } = mountRemote({ events: [] });

    await waitFor(() => expect(onEvents).toHaveBeenCalledWith(fresh));
    // The row is nowhere on screen: it only appears once the owner of the feed
    // hands it back through `events`.
    expect(screen.queryByText('PR opened')).not.toBeInTheDocument();
  });

  it('resumes from the last consumed offset instead of refetching the log', async () => {
    vi.useFakeTimers();
    streamRemoteEvents.mockResolvedValue([event({ offset: 12 })]);
    render(
      <ActivityPanel
        events={[]}
        remote={{ run: RUN, machineName: 'gpu-box', onEvents: vi.fn() }}
        terminal={false}
        open
        onOpenChange={() => {}}
      />,
    );

    await act(async () => {});
    expect(streamRemoteEvents).toHaveBeenLastCalledWith('m-1', 'r-1', 0);

    await act(async () => { vi.advanceTimersByTime(REMOTE_TAIL_POLL_MS); });
    expect(streamRemoteEvents).toHaveBeenLastCalledWith('m-1', 'r-1', 12);
  });

  it('says a local run backfills nothing, rather than "waiting"', () => {
    render(
      <ActivityPanel events={[]} remote={null} terminal={false} open onOpenChange={() => {}} />,
    );

    expect(screen.getByText(/Nothing since this run was opened/)).toBeInTheDocument();
    expect(streamRemoteEvents).not.toHaveBeenCalled();
  });
});

describe('ActivityPanel — the sync affordance', () => {
  it('names the interval the tail actually polls at', async () => {
    mountRemote();

    await waitFor(() => expect(streamRemoteEvents).toHaveBeenCalled());
    expect(screen.getByTestId('activity-sync')).toHaveTextContent(
      `every ${REMOTE_TAIL_POLL_MS / 1000}s`,
    );
  });

  it('stops polling when closed and says the tail is paused', async () => {
    const onOpenChange = vi.fn();
    const { rerender } = mountRemote({ onOpenChange });

    await waitFor(() => expect(streamRemoteEvents).toHaveBeenCalled());
    await userEvent.click(screen.getByTestId('disclosure-trigger'));
    expect(onOpenChange).toHaveBeenCalledWith(false);

    streamRemoteEvents.mockClear();
    vi.useFakeTimers();
    rerender(
      <ActivityPanel
        events={[]}
        remote={{ run: RUN, machineName: 'gpu-box', onEvents: vi.fn() }}
        terminal={false}
        open={false}
        onOpenChange={onOpenChange}
      />,
    );

    await act(async () => { vi.advanceTimersByTime(REMOTE_TAIL_POLL_MS * 4); });
    expect(streamRemoteEvents).not.toHaveBeenCalled();
    expect(screen.getByTestId('activity-sync')).toHaveTextContent('paused');
  });

  it('does not call a closed local panel paused — nothing stopped', () => {
    render(
      <ActivityPanel
        events={[event()]}
        remote={null}
        terminal={false}
        open={false}
        onOpenChange={() => {}}
      />,
    );

    expect(screen.getByTestId('activity-sync')).toHaveTextContent('live');
  });

  it('fetches a finished run once and never schedules a tail for it', async () => {
    vi.useFakeTimers();
    render(
      <ActivityPanel
        events={[]}
        remote={{ run: RUN, machineName: 'gpu-box', onEvents: vi.fn() }}
        terminal
        open
        onOpenChange={() => {}}
      />,
    );

    await act(async () => {});
    expect(streamRemoteEvents).toHaveBeenCalledTimes(1);

    await act(async () => { vi.advanceTimersByTime(REMOTE_TAIL_POLL_MS * 5); });
    expect(streamRemoteEvents).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('activity-sync')).toHaveTextContent('final');
  });

  it('reports one dropped poll quietly and a streak as a disconnection', async () => {
    vi.useFakeTimers();
    streamRemoteEvents.mockRejectedValue(new Error('tunnel down'));
    render(
      <ActivityPanel
        events={[]}
        remote={{ run: RUN, machineName: 'gpu-box', onEvents: vi.fn() }}
        terminal={false}
        open
        onOpenChange={() => {}}
      />,
    );

    await act(async () => {});
    expect(screen.getByTestId('activity-sync')).toHaveTextContent('reconnecting');
    expect(screen.queryByText(/Lost the connection/)).not.toBeInTheDocument();

    for (let i = 0; i < 2; i += 1) {
      await act(async () => { vi.advanceTimersByTime(REMOTE_TAIL_POLL_MS); });
    }

    expect(screen.getByTestId('activity-sync')).toHaveTextContent('disconnected');
    expect(screen.getByText(/Lost the connection to gpu-box/)).toBeInTheDocument();
  });

  it('pulses the dot only while the feed is advancing', async () => {
    const { container, rerender } = mountRemote();
    await waitFor(() => expect(streamRemoteEvents).toHaveBeenCalled());

    const dot = () => container.querySelector('[data-testid="activity-sync"] span');
    expect(dot()).toHaveClass('animate-pulse');
    // §5's motion budget: the pulse must be reachable by the reduced-motion
    // request, and it must be on the dot rather than the label beside it.
    expect(dot()).toHaveClass('motion-reduce:animate-none');
    expect(screen.getByTestId('activity-sync').className).not.toMatch(/animate-pulse/);

    rerender(
      <ActivityPanel
        events={[]}
        remote={{ run: RUN, machineName: 'gpu-box', onEvents: vi.fn() }}
        terminal
        open
        onOpenChange={() => {}}
      />,
    );
    expect(dot()).not.toHaveClass('animate-pulse');
  });
});
