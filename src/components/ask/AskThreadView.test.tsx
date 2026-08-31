// AskThreadView — Acceptance Criteria 2 and 7: selecting a thread renders
// the header/transcript/composer/canvas pane for it, and a thread with no
// messages renders exactly the three `Empty.html` `.try` chips, verbatim,
// with no chip naming a run, ticket id, or failure.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { listen, type Event, type EventCallback } from '@tauri-apps/api/event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const listAskThreads = vi.fn();
const loadAskThread = vi.fn();
const askTurnRunning = vi.fn();
const sendAskTurn = vi.fn();

vi.mock('../../lib/ask', () => ({
  listAskThreads: (...args: unknown[]) => listAskThreads(...args),
  loadAskThread: (...args: unknown[]) => loadAskThread(...args),
  askTurnRunning: (...args: unknown[]) => askTurnRunning(...args),
  sendAskTurn: (...args: unknown[]) => sendAskTurn(...args),
  EVENT_ASK_AGENT_EVENT: 'ask_agent_event',
  EVENT_ASK_TURN_STATUS: 'ask_turn_status',
  EVENT_ASK_TURN_COMPLETED: 'ask_turn_completed',
}));

vi.mock('./NewAskThreadModal', () => ({
  NewAskThreadModal: ({
    seedTitle,
    onCreated,
  }: {
    seedTitle: string;
    onCreated: (created: AskThread) => void;
  }) => (
    <div data-testid="new-ask-thread-modal-stub">
      <span data-testid="new-ask-thread-seed">{seedTitle}</span>
      <button type="button" data-testid="new-ask-thread-create" onClick={() => onCreated(CREATED)}>
        Start thread
      </button>
    </div>
  ),
}));

vi.mock('./AskThreadSettingsPanel', () => ({
  AskThreadSettingsPanel: () => <div data-testid="ask-thread-settings-panel-stub" />,
}));

import { AskThreadView } from './AskThreadView';
import type { AskMessageView, AskThread, AskThreadDetail } from '../../types';

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
    turn_count: 0,
    cost_usd: 0,
    tokens: 0,
    network: true,
    created_at: 0,
    updated_at: 0,
    ...overrides,
  };
}

/** What the stubbed modal reports back from a creation. Read at click time,
 *  not when the factory above runs, so it is still in TDZ-safe territory. */
const CREATED = thread({ id: 't-new', title: 'Draw the architecture of crates/demeteo-core' });

function message(overrides: Partial<AskMessageView> = {}): AskMessageView {
  return {
    id: 'm1',
    thread_id: 't1',
    role: 'assistant',
    text: 'The answer.',
    cost_usd: null,
    tokens: null,
    turn_activity: null,
    canvas_paths: null,
    checked_commit_sha: null,
    created_at: 0,
    prose: 'The answer.',
    canvas: null,
    canvas_error: null,
    ...overrides,
  };
}

function detail(t: AskThread, messages: AskMessageView[] = []): AskThreadDetail {
  return { thread: t, messages };
}

/** The handlers the view and `useAskStream` registered, by event name. */
const listeners = new Map<string, Array<EventCallback<unknown>>>();

/** Deliver a backend event to every handler listening for it, inside `act` —
 *  a status event lands in `setState`, an agent event in the stream store. */
function emit(event: string, payload: unknown) {
  const handlers = listeners.get(event) ?? [];
  if (handlers.length === 0) throw new Error(`no listener registered for "${event}"`);
  act(() => {
    for (const handler of [...handlers]) handler({ event, id: 1, payload } as Event<unknown>);
  });
}

/** Hand every `listen(...)` to {@link emit} instead of the no-op stub. */
function captureListeners() {
  listeners.clear();
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    const bucket = listeners.get(String(event)) ?? [];
    bucket.push(handler as EventCallback<unknown>);
    listeners.set(String(event), bucket);
    return () => {
      listeners.set(
        String(event),
        (listeners.get(String(event)) ?? []).filter((h) => h !== handler),
      );
    };
  });
}

beforeEach(() => {
  askTurnRunning.mockResolvedValue(false);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe('AskThreadView — empty state', () => {
  it('renders exactly the three Empty.html chips, verbatim, for a thread with no messages', async () => {
    listAskThreads.mockResolvedValue([thread()]);
    loadAskThread.mockResolvedValue(detail(thread(), []));

    render(<AskThreadView projectId="p1" machineId="local" />);

    const chips = await screen.findAllByTestId('ask-try-chip');
    expect(chips).toHaveLength(3);
    expect(chips[0]).toHaveTextContent('Draw the architecture of');
    expect(chips[0]).toHaveTextContent('crates/demeteo-core');
    expect(chips[1]).toHaveTextContent('Map the journey from New Feature to a merged branch');
    expect(chips[2]).toHaveTextContent('What changed in Tauri v2 capabilities since 2.1?');
    expect(chips[2]).toHaveTextContent('web');
  });

  it('never mentions a specific run, ticket id, or failure in a chip', async () => {
    listAskThreads.mockResolvedValue([thread()]);
    loadAskThread.mockResolvedValue(detail(thread(), []));

    render(<AskThreadView projectId="p1" machineId="local" />);

    await screen.findAllByTestId('ask-try-chip');
    expect(screen.queryByText(/f-2291/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/wedge/i)).not.toBeInTheDocument();
  });

  it('seeds the composer with a chip’s text without sending', async () => {
    listAskThreads.mockResolvedValue([thread()]);
    loadAskThread.mockResolvedValue(detail(thread(), []));

    render(<AskThreadView projectId="p1" machineId="local" />);

    const chips = await screen.findAllByTestId('ask-try-chip');
    fireEvent.click(chips[1]);

    const input = (await screen.findByTestId('ask-composer')) as HTMLInputElement;
    await waitFor(() => expect(input.value).toBe('Map the journey from New Feature to a merged branch'));
    expect(sendAskTurn).not.toHaveBeenCalled();
  });
});

/**
 * A chip clicked with no thread open. It is the only affordance the first-run
 * empty state has, so the text it carries has to survive the modal that stands
 * between the click and a composer to type it into.
 *
 * Both were watched to fail first, against a `pickChip` that opened the modal
 * and returned: the seed reached neither the modal's name field nor the
 * composer, which came up empty with the question thrown away.
 */
describe('AskThreadView — a Try chip clicked with no thread open', () => {
  it('carries the chip’s text into the thread the modal creates', async () => {
    listAskThreads.mockResolvedValue([]);
    loadAskThread.mockResolvedValue(detail(CREATED, []));

    render(<AskThreadView projectId="p1" machineId="local" />);

    const chips = await screen.findAllByTestId('ask-try-chip');
    fireEvent.click(chips[0]);

    fireEvent.click(await screen.findByTestId('new-ask-thread-create'));

    const input = (await screen.findByTestId('ask-composer')) as HTMLInputElement;
    await waitFor(() =>
      expect(input.value).toBe('Draw the architecture of crates/demeteo-core'),
    );
    expect(sendAskTurn).not.toHaveBeenCalled();
  });

  it('offers the chip’s text as the new thread’s name', async () => {
    listAskThreads.mockResolvedValue([]);

    render(<AskThreadView projectId="p1" machineId="local" />);

    const chips = await screen.findAllByTestId('ask-try-chip');
    fireEvent.click(chips[1]);

    expect(await screen.findByTestId('new-ask-thread-seed')).toHaveTextContent(
      'Map the journey from New Feature to a merged branch',
    );
  });

  it('starts a thread opened from the header button on an empty name', async () => {
    listAskThreads.mockResolvedValue([]);

    render(<AskThreadView projectId="p1" machineId="local" />);

    fireEvent.click(await screen.findByTestId('ask-new-thread'));

    expect(screen.getByTestId('new-ask-thread-seed')).toBeEmptyDOMElement();
  });
});

describe('AskThreadView — thread selection', () => {
  it('selects the most recently touched open thread and renders header + transcript + canvas pane', async () => {
    const open = thread({ id: 'open-1', title: 'Open thread', status: 'open', turn_count: 2 });
    const closed = thread({ id: 'closed-1', title: 'Closed thread', status: 'closed' });
    listAskThreads.mockResolvedValue([open, closed]);
    loadAskThread.mockResolvedValue(detail(open, [message({ thread_id: 'open-1' })]));

    render(<AskThreadView projectId="p1" machineId="local" />);

    expect(await screen.findByText('Open thread')).toBeInTheDocument();
    expect(loadAskThread).toHaveBeenCalledWith('open-1');
    expect(await screen.findByTestId('ask-transcript')).toBeInTheDocument();
    expect(screen.getByTestId('ask-composer')).toBeInTheDocument();
  });

  it('renders a project-level empty state when there is no open thread', async () => {
    listAskThreads.mockResolvedValue([]);

    render(<AskThreadView projectId="p1" machineId="local" />);

    expect(await screen.findByRole('heading', { name: 'New thread' })).toBeInTheDocument();
    expect(screen.getAllByTestId('ask-try-chip')).toHaveLength(3);
    expect(screen.getByTestId('ask-canvas-placeholder')).toBeInTheDocument();
    expect(screen.queryByTestId('ask-composer')).not.toBeInTheDocument();
  });
});

/**
 * A turn already under way when this surface mounts — the user navigated away
 * and back, or reloaded the window, part-way through a multi-minute answer.
 * `ask_turn_status` fires on transitions only, so the one that opened the turn
 * is never repeated and `ask_running` is the only thing that still knows.
 *
 * All three were watched to fail first. Against a view that queried nothing
 * on select, the running case drew no streaming bubble and left the composer
 * enabled — its next Send would come back `ALREADY_RUNNING` — and the idle
 * case failed on the query it never made. The third fails whenever the answer
 * is applied without re-checking the stream: a turn that ends mid-flight is
 * re-opened by a stale `true` and nothing ever closes it again.
 */
describe('AskThreadView — a turn already running when the surface mounts', () => {
  it('draws the running turn and holds the composer', async () => {
    const t = thread();
    listAskThreads.mockResolvedValue([t]);
    loadAskThread.mockResolvedValue(detail(t, [message({ role: 'user', text: 'Why?', prose: 'Why?' })]));
    askTurnRunning.mockResolvedValue(true);

    render(<AskThreadView projectId="p1" machineId="local" />);

    expect(await screen.findByTestId('ask-streaming-bubble')).toBeInTheDocument();
    expect(askTurnRunning).toHaveBeenCalledWith('t1');
    expect(screen.getByTestId('ask-composer')).toBeDisabled();
    expect(screen.queryByTestId('ask-canvas-placeholder')).not.toBeInTheDocument();
  });

  it('yields to the stream when the turn ends before the query answers', async () => {
    captureListeners();
    const t = thread();
    listAskThreads.mockResolvedValue([t]);
    loadAskThread.mockResolvedValue(detail(t, [message({ role: 'user', text: 'Why?', prose: 'Why?' })]));
    let answer: (running: boolean) => void = () => {};
    askTurnRunning.mockReturnValue(
      new Promise<boolean>((resolve) => {
        answer = resolve;
      }),
    );

    render(<AskThreadView projectId="p1" machineId="local" />);
    await screen.findByTestId('ask-transcript');
    await waitFor(() => expect(listeners.get('ask_turn_status')?.length ?? 0).toBeGreaterThan(0));

    // The turn this read was about ends while its answer is still in flight.
    emit('ask_turn_status', { thread_id: 't1', status: 'idle', reason: null });
    await act(async () => {
      answer(true);
    });

    expect(screen.queryByTestId('ask-streaming-bubble')).not.toBeInTheDocument();
    expect(screen.getByTestId('ask-composer')).not.toBeDisabled();
  });

  it('leaves a thread with no turn running exactly as it was', async () => {
    const t = thread();
    listAskThreads.mockResolvedValue([t]);
    loadAskThread.mockResolvedValue(detail(t, [message({ role: 'user', text: 'Why?', prose: 'Why?' })]));
    askTurnRunning.mockResolvedValue(false);

    render(<AskThreadView projectId="p1" machineId="local" />);

    expect(await screen.findByTestId('ask-transcript')).toBeInTheDocument();
    await waitFor(() => expect(askTurnRunning).toHaveBeenCalledWith('t1'));
    expect(screen.queryByTestId('ask-streaming-bubble')).not.toBeInTheDocument();
    expect(screen.getByTestId('ask-composer')).not.toBeDisabled();
  });
});

/**
 * A turn that ends without ever emitting `ask_turn_completed` —
 * `application::ask::turn::announced()`'s prepare-failure branch, which drops
 * the claim and emits a terminal `error` status when `worktree::resolve`/
 * `ensure` fails. `run()` is never reached, so the completed handler that
 * calls `end` never fires.
 *
 * Both cases below were watched to fail first, against the handler as it
 * stood: it cleared `phase` and left the `LiveTurn` in place, so the retry
 * inherited the failed attempt's ledger and its clock.
 */
describe('AskThreadView — a turn that ends without completing', () => {
  beforeEach(captureListeners);

  afterEach(() => {
    vi.useRealTimers();
  });

  async function openThread(t: AskThread = thread()) {
    listAskThreads.mockResolvedValue([t]);
    loadAskThread.mockResolvedValue(detail(t, []));
    render(<AskThreadView projectId="p1" machineId="local" />);
    await screen.findAllByTestId('ask-try-chip');
    // The header's `AskThreadSwitcher` mounts with the thread and loads its
    // own copy of the list; letting that land here keeps its `setState` out
    // of the emissions below.
    await waitFor(() => expect(listAskThreads).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(listeners.get('ask_turn_status')?.length ?? 0).toBeGreaterThan(0));
  }

  it('drops the thread’s live turn on a terminal error status', async () => {
    await openThread();

    emit('ask_turn_status', { thread_id: 't1', status: 'running', reason: null });
    await act(async () => {
      emit('ask_agent_event', {
        thread_id: 't1',
        event: { kind: 'tool_call', tool_call_id: 'c1', action: 'Read', target: 'src/lib/ask.ts' },
      });
      emit('ask_agent_event', { thread_id: 't1', event: { kind: 'text', delta: 'Reading the repo' } });
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    expect(screen.getByTestId('turn-activity-summary')).toHaveTextContent('1 read');

    emit('ask_turn_status', { thread_id: 't1', status: 'error', reason: 'worktree is gone' });
    expect(screen.queryByTestId('turn-activity')).not.toBeInTheDocument();

    // The next turn on this thread reads the store fresh: nothing of the
    // failed attempt's fold survives it.
    emit('ask_turn_status', { thread_id: 't1', status: 'setting_up', reason: null });
    expect(screen.getByTestId('turn-activity')).toHaveTextContent('Preparing the turn');
    expect(screen.queryByTestId('turn-activity-summary')).not.toBeInTheDocument();
  });

  it('restarts the elapsed clock from the retry, not from the failed attempt', async () => {
    await openThread();

    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-01-01T10:00:00Z'));

    emit('ask_turn_status', { thread_id: 't1', status: 'setting_up', reason: null });
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByTestId('turn-elapsed').textContent).toBe('1s');

    emit('ask_turn_status', { thread_id: 't1', status: 'error', reason: 'host unreachable' });

    // 45 minutes later the host is back and the user retries. The clock is
    // re-read past the tick the failed attempt's interval was still holding,
    // so a retained `startedAt` would show the whole gap here.
    vi.setSystemTime(new Date('2026-01-01T10:45:00Z'));
    emit('ask_turn_status', { thread_id: 't1', status: 'setting_up', reason: null });
    act(() => {
      vi.advanceTimersByTime(2000);
    });

    // `toHaveTextContent` matches a substring, and "45m 2s" ends in "2s".
    expect(screen.getByTestId('turn-elapsed').textContent).toBe('2s');
  });
});

/**
 * A thread left mid-turn when the user opens another one. `useAskStream`'s
 * store is keyed by `thread_id` and folds every `ask_agent_event` whatever is
 * on screen, so the two events that *close* a turn have to be just as
 * thread-keyed: a guard on the selected thread strands the entry, and the next
 * question asked on that thread inherits it.
 *
 * All three were watched to fail first, against the `thread_id !== selectedId`
 * early-return both handlers opened with. Turn 2's bubble came up carrying
 * turn 1's prose, its Sources list, its `2 reads` summary and a clock counting
 * from turn 1's start — indistinguishable from a live answer.
 */
describe('AskThreadView — a turn that ends on a thread the user navigated away from', () => {
  beforeEach(captureListeners);

  afterEach(() => {
    vi.useRealTimers();
  });

  const A = thread({ id: 'a', title: 'Thread A' });
  const B = thread({ id: 'b', title: 'Thread B' });

  async function openFromSwitcher(title: string) {
    fireEvent.click(screen.getByTestId('ask-thread-switcher-trigger'));
    const rows = await screen.findAllByTestId('ask-thread-switcher-row');
    const row = rows.find((candidate) => candidate.textContent?.includes(title));
    if (row === undefined) throw new Error(`no switcher row for "${title}"`);
    fireEvent.click(row);
  }

  /** Thread A answering, with prose, a source and two reads folded in — then
   *  the user opens B while it is still running. */
  async function runTurnOnAThenOpenB() {
    listAskThreads.mockResolvedValue([A, B]);
    loadAskThread.mockImplementation(async (id: string) =>
      id === 'a'
        ? detail(A, [message({ id: 'ma', thread_id: 'a', role: 'user', text: 'Why?', prose: 'Why?' })])
        : detail(B, [message({ id: 'mb', thread_id: 'b', role: 'user', text: 'Where?', prose: 'Where?' })]),
    );

    render(<AskThreadView projectId="p1" machineId="local" />);
    await screen.findByTestId('ask-transcript');
    await waitFor(() => expect(listeners.get('ask_turn_status')?.length ?? 0).toBeGreaterThan(0));

    emit('ask_turn_status', { thread_id: 'a', status: 'running', reason: null });
    await act(async () => {
      emit('ask_agent_event', {
        thread_id: 'a',
        event: { kind: 'tool_call', tool_call_id: 'c1', action: 'Read', target: 'src/lib/ask.ts' },
      });
      emit('ask_agent_event', {
        thread_id: 'a',
        event: {
          kind: 'tool_call',
          tool_call_id: 'c2',
          action: 'Read',
          target: 'https://tauri.app/v2/security/capabilities/',
        },
      });
      emit('ask_agent_event', { thread_id: 'a', event: { kind: 'text', delta: 'Capabilities gained a field.' } });
      await new Promise((resolve) => requestAnimationFrame(resolve));
    });
    // The prose rides `useThrottledValue`'s 250ms budget, so it lands a beat
    // after the delta that carried it.
    await waitFor(() =>
      expect(screen.getByTestId('ask-streaming-bubble')).toHaveTextContent('Capabilities gained a field.'),
    );
    const bubble = within(screen.getByTestId('ask-streaming-bubble'));
    expect(bubble.getByTestId('turn-activity-summary')).toHaveTextContent('2 reads');
    expect(bubble.getAllByTestId('ask-source')).toHaveLength(1);

    await openFromSwitcher('Thread B');
    await waitFor(() => expect(loadAskThread).toHaveBeenLastCalledWith('b'));
  }

  /** Back on A, ask again — and assert the bubble is about *this* question. */
  async function askAgainOnA() {
    await openFromSwitcher('Thread A');
    await waitFor(() => expect(loadAskThread).toHaveBeenLastCalledWith('a'));

    const composer = (await screen.findByTestId('ask-composer')) as HTMLInputElement;
    expect(composer).not.toBeDisabled();
    fireEvent.change(composer, { target: { value: 'And now?' } });
    sendAskTurn.mockResolvedValue(
      message({ id: 'm2', thread_id: 'a', role: 'user', text: 'And now?', prose: 'And now?' }),
    );

    // Faked only from here: turn 1's start was stamped off the real clock, so
    // a retained one shows the whole 45-minute gap where a fresh one shows the
    // tick. Everything above needs real timers — `waitFor` polls on one.
    vi.useFakeTimers();
    vi.setSystemTime(Date.now() + 45 * 60 * 1000);

    await act(async () => {
      fireEvent.click(screen.getByTestId('ask-composer-send'));
    });
    emit('ask_turn_status', { thread_id: 'a', status: 'setting_up', reason: null });

    const bubble = within(screen.getByTestId('ask-streaming-bubble'));
    expect(screen.getByTestId('ask-streaming-bubble')).not.toHaveTextContent('Capabilities gained a field.');
    expect(bubble.queryByTestId('ask-sources')).not.toBeInTheDocument();
    expect(bubble.queryByTestId('turn-activity-summary')).not.toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(2000);
    });
    expect(bubble.getByTestId('turn-elapsed').textContent).toBe('2s');
  }

  it('ends the turn on `ask_turn_completed` for a thread that is not selected', async () => {
    await runTurnOnAThenOpenB();

    emit('ask_turn_completed', {
      thread_id: 'a',
      title: 'Thread A',
      message_id: 'm1',
      ending: 'success',
      reason: null,
      cost_usd: 0,
      tokens: 0,
      duration_ms: 1000,
    });

    await askAgainOnA();
  });

  it('ends the turn on a terminal `ask_turn_status` for a thread that is not selected', async () => {
    await runTurnOnAThenOpenB();

    emit('ask_turn_status', { thread_id: 'a', status: 'error', reason: 'worktree is gone' });

    // The reason belongs to the thread it names, not to the one on screen —
    // `setError` is what stays behind the selection guard.
    expect(screen.queryByText('worktree is gone')).not.toBeInTheDocument();
    expect(screen.getByTestId('ask-composer')).not.toBeDisabled();

    await askAgainOnA();
  });
});
