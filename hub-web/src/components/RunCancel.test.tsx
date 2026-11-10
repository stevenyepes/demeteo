import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RunCancel } from './RunCancel';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const FEATURE_ID = 'f-42';
const REQUEST_ID = 'q7Zr-1_x';
const CANCEL_PATH = `/v1/instances/${INSTANCE_ID}/runs/${FEATURE_ID}/cancel`;
const STATUS_PATH = `/v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;

const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;

type Sent = { method: string; path: string; body: unknown };
type Reply = { status: number; body: unknown } | Promise<Response>;

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
      kind: 'cancel_feature',
      status,
      deliveries: 1,
      expires_at: EXPIRES_AT,
      answered_at: null,
    },
  };
}

function renderCancel(overrides: Partial<ComponentProps<typeof RunCancel>> = {}) {
  const props = { instanceId: INSTANCE_ID, featureId: FEATURE_ID, status: 'running' as const, scopes: ['read', 'spend'] as ComponentProps<typeof RunCancel>['scopes'], ...overrides };
  const view = render(<RunCancel {...props} />);
  return { ...view, rerenderWith: (next: Partial<ComponentProps<typeof RunCancel>>) => view.rerender(<RunCancel {...props} {...next} />) };
}

async function advance(ms: number) {
  await act(() => vi.advanceTimersByTimeAsync(ms));
}

function cancelButton(): HTMLElement {
  return screen.getByRole('button', { name: 'Cancel run' });
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
  expect.soft(strays).toEqual([]);
});

describe('RunCancel', () => {
  it('sends nothing until the cancel is confirmed, then one empty POST and its status', async () => {
    const { sent } = stubHub({
      [`POST ${CANCEL_PATH}`]: [ticket],
      [`GET ${STATUS_PATH}`]: [requestState('queued'), requestState('accepted')],
    });
    renderCancel();

    fireEvent.click(cancelButton());
    await advance(0);
    expect(sent).toEqual([]);
    expect(screen.getByRole('group', { name: 'Confirm cancel' })).toHaveTextContent('Every step stops');

    fireEvent.click(cancelButton());
    await advance(0);
    expect(sent[0]).toEqual({ method: 'POST', path: CANCEL_PATH, body: {} });
    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Queued at the Hub');

    await advance(1000);
    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Accepted by the instance');
  });

  it('sends nothing when the user keeps the run going', async () => {
    const { sent } = stubHub({});
    renderCancel();

    fireEvent.click(cancelButton());
    fireEvent.click(screen.getByRole('button', { name: 'Keep running' }));
    await advance(0);

    expect(sent).toEqual([]);
    expect(cancelButton()).toBeEnabled();
    expect(screen.queryByRole('group', { name: 'Confirm cancel' })).toBeNull();
  });

  it('sends one request however often the confirm is clicked while one is in flight', async () => {
    let answer: (response: Response) => void = () => {};
    const held = new Promise<Response>((resolve) => {
      answer = resolve;
    });
    const { sent } = stubHub({ [`POST ${CANCEL_PATH}`]: [held] });
    renderCancel();

    fireEvent.click(cancelButton());
    fireEvent.click(cancelButton());
    fireEvent.click(screen.getByRole('button', { name: 'Cancelling…' }));
    await advance(0);

    expect(sent).toHaveLength(1);
    answer(new Response(JSON.stringify({ error: 'instance_offline' }), { status: 409 }));
    await advance(0);
    expect(screen.getByRole('alert')).toHaveTextContent('instance_offline');
  });

  it('is disabled, saying why, when the instance has not granted spend', () => {
    stubHub({});
    renderCancel({ scopes: ['read'] });

    expect(cancelButton()).toBeDisabled();
    expect(screen.getByText(/Spend is off on this instance/)).toBeInTheDocument();
  });

  it('renders the Hub refusing the scope when it arrives anyway', async () => {
    stubHub({ [`POST ${CANCEL_PATH}`]: [{ status: 409, body: { error: 'scope_not_granted', scope: 'spend' } }] });
    renderCancel({ scopes: null });

    fireEvent.click(cancelButton());
    fireEvent.click(cancelButton());
    await advance(0);

    expect(screen.getByRole('alert')).toHaveTextContent(/^scope_not_granted$/);
    expect(screen.getByText('The Hub says spend is off on this instance.')).toBeVisible();
    expect(screen.getByText(/demeteo-runner hub policy --spend on/)).toBeVisible();
  });

  it.each(['completed', 'failed', 'cancelled'] as const)('offers nothing on a run that is %s', (status) => {
    stubHub({});
    renderCancel({ status });

    expect(screen.queryByRole('button', { name: 'Cancel run' })).toBeNull();
  });

  it('keeps the answer on screen once the run it cancelled reads as cancelled', async () => {
    stubHub({
      [`POST ${CANCEL_PATH}`]: [ticket],
      [`GET ${STATUS_PATH}`]: [requestState('accepted')],
    });
    const { rerenderWith } = renderCancel();

    fireEvent.click(cancelButton());
    fireEvent.click(cancelButton());
    await advance(0);
    rerenderWith({ status: 'cancelled' });
    await advance(0);

    expect(screen.getByRole('status', { name: 'Request status' })).toHaveTextContent('Accepted by the instance');
  });
});
