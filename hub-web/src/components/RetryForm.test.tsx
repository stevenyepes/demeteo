import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RetryForm } from './RetryForm';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const FEATURE_ID = 'f-42';
const STEP_ID = 'implement';
const ASSIGN_ID = 'assign-1';
const RETRY_ID = 'retry-1';
const STEP_PATH = `/v1/instances/${INSTANCE_ID}/runs/${FEATURE_ID}/steps/${STEP_ID}`;
const statusPath = (requestId: string) => `/v1/instances/${INSTANCE_ID}/requests/${requestId}`;

/** Unix seconds; the fake clock starts here. */
const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;

type Sent = { method: string; path: string; body: unknown };
type Reply = { status: number; body: unknown };

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
      return new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
  return { sent };
}

function ticket(requestId: string): Reply {
  return { status: 202, body: { request_id: requestId, expires_at: EXPIRES_AT } };
}

function requestState(requestId: string, kind: string, status: string, extra: Record<string, unknown> = {}): Reply {
  return {
    status: 200,
    body: {
      request_id: requestId,
      kind,
      status,
      ...extra,
      deliveries: 1,
      expires_at: EXPIRES_AT,
      answered_at: null,
    },
  };
}

function renderForm(overrides: Partial<ComponentProps<typeof RetryForm>> = {}) {
  return render(
    <RetryForm
      instanceId={INSTANCE_ID}
      featureId={FEATURE_ID}
      stepId={STEP_ID}
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

describe('RetryForm', () => {
  it('with nothing changed, queues only the retry', async () => {
    const { sent } = stubHub({
      [`POST ${STEP_PATH}/retry`]: [ticket(RETRY_ID)],
      [`GET ${statusPath(RETRY_ID)}`]: [requestState(RETRY_ID, 'retry_step', 'accepted')],
    });
    renderForm();

    fireEvent.click(screen.getByRole('button', { name: 'Retry step' }));
    await advance(0);

    expect(sent.map(({ method, path }) => `${method} ${path}`)).toEqual([
      `POST ${STEP_PATH}/retry`,
      `GET ${statusPath(RETRY_ID)}`,
    ]);
    expect(sent[0]?.body).toEqual({});
    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Accepted by the instance');
  });

  it('queues the retry only once the new assignment is accepted', async () => {
    const { sent } = stubHub({
      [`POST ${STEP_PATH}/assignment`]: [ticket(ASSIGN_ID)],
      [`GET ${statusPath(ASSIGN_ID)}`]: [
        requestState(ASSIGN_ID, 'set_step_assignment', 'delivered'),
        requestState(ASSIGN_ID, 'set_step_assignment', 'accepted'),
      ],
      [`POST ${STEP_PATH}/retry`]: [ticket(RETRY_ID)],
      [`GET ${statusPath(RETRY_ID)}`]: [requestState(RETRY_ID, 'retry_step', 'queued')],
    });
    renderForm();

    choose('Agent', 'codex');
    fireEvent.click(screen.getByRole('button', { name: 'Reassign and retry' }));
    await advance(0);

    expect(sent.map(({ path }) => path)).toEqual([`${STEP_PATH}/assignment`, statusPath(ASSIGN_ID)]);
    expect(sent[0]?.body).toEqual({ agent_kind: 'codex' });

    await advance(1000);

    expect(sent.map(({ path }) => path)).toEqual([
      `${STEP_PATH}/assignment`,
      statusPath(ASSIGN_ID),
      statusPath(ASSIGN_ID),
      `${STEP_PATH}/retry`,
      statusPath(RETRY_ID),
    ]);
  });

  it('sends no retry when the assignment is refused', async () => {
    const { sent } = stubHub({
      [`POST ${STEP_PATH}/assignment`]: [ticket(ASSIGN_ID)],
      [`GET ${statusPath(ASSIGN_ID)}`]: [
        requestState(ASSIGN_ID, 'set_step_assignment', 'refused', { reason: 'harness_not_registered' }),
      ],
    });
    renderForm();

    choose('Effort', 'high');
    fireEvent.click(screen.getByRole('button', { name: 'Reassign and retry' }));
    await advance(0);
    await advance(10_000);

    expect(sent.map(({ path }) => path)).toEqual([`${STEP_PATH}/assignment`, statusPath(ASSIGN_ID)]);
    expect(screen.getByText('The assignment did not take, so the retry was not sent.')).toBeInTheDocument();
  });

  it('is held without spend and offers no assignment on a step that runs no agent', () => {
    stubHub({});
    const { unmount } = renderForm({ scopes: ['read'] });

    expect(screen.getByRole('button', { name: 'Retry step' })).toBeDisabled();
    expect(screen.getByText(/needs the/)).toHaveTextContent('Retrying a step needs the spend scope');
    unmount();

    renderForm({ kind: 'command' });
    expect(screen.queryByLabelText('Agent')).toBeNull();
    expect(screen.getByRole('button', { name: 'Retry step' })).toBeEnabled();
  });
});
