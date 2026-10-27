import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AssignmentForm } from './AssignmentForm';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const FEATURE_ID = 'f-42';
const STEP_ID = 'implement';
const REQUEST_ID = 'q7Zr-1_x';
const ASSIGNMENT_PATH = `/v1/instances/${INSTANCE_ID}/runs/${FEATURE_ID}/steps/${STEP_ID}/assignment`;
const STATUS_PATH = `/v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;

/** Unix seconds; the fake clock starts here. */
const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;

type Sent = { method: string; path: string; body: unknown };
type Reply = { status: number; body: unknown } | Promise<Response>;

/** Every request the double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Answers each `"METHOD path"` with its replies in order; anything else, or one call too many, throws. */
function stubHub(script: Record<string, Reply[]>) {
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const request = `${method} ${String(input)}`;
      const reply = script[request]?.shift();
      if (reply === undefined) {
        unscripted.push(request);
        throw new Error(`fetch double was not told about ${request} here`);
      }
      sent.push({
        method,
        path: String(input),
        body: typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body,
      });
      if (reply instanceof Promise) return reply;
      return new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
  return { sent };
}

const ticket = { status: 202, body: { request_id: REQUEST_ID, expires_at: EXPIRES_AT } };

function requestState(status: string) {
  return {
    status: 200,
    body: {
      request_id: REQUEST_ID,
      kind: 'set_step_assignment',
      status,
      deliveries: 1,
      expires_at: EXPIRES_AT,
      answered_at: null,
    },
  };
}

function renderForm(overrides: Partial<ComponentProps<typeof AssignmentForm>> = {}) {
  return render(
    <AssignmentForm
      instanceId={INSTANCE_ID}
      featureId={FEATURE_ID}
      stepId={STEP_ID}
      runStatus="running"
      state="completed"
      kind="agent"
      scopes={['read', 'spend']}
      {...overrides}
    />,
  );
}

async function advance(ms: number) {
  await act(() => vi.advanceTimersByTimeAsync(ms));
}

function choose(label: string, value: string) {
  fireEvent.change(screen.getByLabelText(label), { target: { value } });
}

function submitButton(): HTMLElement {
  return screen.getByRole('button', { name: 'Change assignment' });
}

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  vi.setSystemTime(NOW * 1000);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
});

describe('AssignmentForm', () => {
  it('posts only the chosen agent and effort to the node, then shows the request status', async () => {
    const { sent } = stubHub({
      [`POST ${ASSIGNMENT_PATH}`]: [ticket],
      [`GET ${STATUS_PATH}`]: [requestState('queued'), requestState('accepted')],
    });
    renderForm();

    choose('Agent', 'codex');
    choose('Effort', 'xhigh');
    fireEvent.click(submitButton());
    await advance(0);

    expect(sent[0]).toEqual({ method: 'POST', path: ASSIGNMENT_PATH, body: { agent_kind: 'codex', effort: 'xhigh' } });
    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Queued at the Hub');

    await advance(1000);

    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Accepted by the instance');
    expect(sent.map((each) => `${each.method} ${each.path}`)).toEqual([
      `POST ${ASSIGNMENT_PATH}`,
      `GET ${STATUS_PATH}`,
      `GET ${STATUS_PATH}`,
    ]);
  });

  it('sends a typed model trimmed, and nothing it was not given', async () => {
    const { sent } = stubHub({ [`POST ${ASSIGNMENT_PATH}`]: [ticket], [`GET ${STATUS_PATH}`]: [requestState('queued')] });
    renderForm();

    fireEvent.change(screen.getByLabelText('Model'), { target: { value: '  opus-x  ' } });
    fireEvent.click(submitButton());
    await advance(0);

    expect(sent[0]?.body).toEqual({ model: 'opus-x' });
  });

  it('offers nothing to send until a field is set', () => {
    stubHub({});
    renderForm();

    expect(submitButton()).toBeDisabled();
    choose('Effort', 'low');
    expect(submitButton()).toBeEnabled();
  });

  it('is disabled with copy naming spend when the instance has not granted it', () => {
    stubHub({});
    renderForm({ scopes: ['read'] });

    expect(screen.getByRole('group')).toBeDisabled();
    expect(screen.getByRole('group')).toHaveAccessibleDescription(/needs the spend scope/);
    expect(screen.getByLabelText('Agent')).toBeDisabled();
    expect(submitButton()).toBeDisabled();
  });

  it('renders the Hub refusing the scope when it arrives anyway', async () => {
    stubHub({ [`POST ${ASSIGNMENT_PATH}`]: [{ status: 409, body: { error: 'scope_not_granted', scope: 'spend' } }] });
    renderForm({ scopes: null });

    choose('Agent', 'opencode');
    fireEvent.click(submitButton());
    await advance(0);

    expect(screen.getByRole('alert')).toHaveTextContent(/^scope_not_granted$/);
    expect(screen.getByText(/has not granted the/)).toHaveTextContent('spend');
    expect(screen.queryByRole('status', { name: 'Request status' })).toBeNull();
  });

  it('sends one request however often it is submitted while one is in flight', async () => {
    let answer: (response: Response) => void = () => {};
    const held = new Promise<Response>((resolve) => {
      answer = resolve;
    });
    const { sent } = stubHub({ [`POST ${ASSIGNMENT_PATH}`]: [held] });
    renderForm();

    choose('Agent', 'pi');
    fireEvent.click(submitButton());
    fireEvent.click(submitButton());
    await advance(0);

    expect(submitButton()).toBeDisabled();
    expect(sent).toHaveLength(1);
    answer(new Response(JSON.stringify({ error: 'instance_offline' }), { status: 409 }));
    await advance(0);
    expect(screen.getByRole('alert')).toHaveTextContent('instance_offline');
  });

  it.each([
    [{ state: 'running' as const }, /this step is running/i],
    [{ kind: 'gate' }, /gate step runs no agent/i],
    [{ kind: 'command' }, /command step runs no agent/i],
    [{ runStatus: 'completed' as const }, /this run is completed/i],
  ])('is disabled with a reason when %o', (overrides, reason) => {
    stubHub({});
    renderForm(overrides);

    expect(screen.getByRole('group')).toBeDisabled();
    expect(screen.getByRole('group')).toHaveAccessibleDescription(reason);
  });

  it('is enabled for a step the mirror reported nothing about on an unreported graph', () => {
    stubHub({});
    renderForm({ state: null, kind: null });

    expect(screen.getByRole('group')).toBeEnabled();
  });
});
