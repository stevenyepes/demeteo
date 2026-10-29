import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TONE_TEXT } from '../../../src/lib/runStatus';
import { RequestStatus } from './RequestStatus';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const REQUEST_ID = 'q7Zr-1_x';
const STATUS_PATH = `/v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;

/** Unix seconds; the fake clock starts here. */
const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;
const TICKET = { requestId: REQUEST_ID, expiresAt: EXPIRES_AT };

const LABELS = [
  'Queued at the Hub',
  'Delivered, waiting for the instance',
  'Accepted by the instance',
  'Refused by the instance',
  'Expired before the instance answered',
];

/** Well past `EXPIRES_AT`, so a poll that would run on past it gets the chance to. */
const LONG_MS = 10 * 60 * 1000;

/** Every request the double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** A reply the Hub refuses with HTTP `status` and the `error` code `code`. */
class Failure {
  constructor(
    readonly status: number,
    readonly code: string,
  ) {}
}

function failure(status: number, code: string): Failure {
  return new Failure(status, code);
}

/** Answers `STATUS_PATH` with `replies` in order; anything else, or one read too many, throws. */
function stubStatus(replies: (Record<string, unknown> | Failure)[]) {
  const pending = [...replies];
  const sent: string[] = [];
  /** Unix seconds of each answered read. */
  const at: number[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || input !== STATUS_PATH || method !== 'GET') {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push(String(input));
    at.push(Date.now() / 1000);
    const failed = next instanceof Failure;
    return new Response(JSON.stringify(failed ? { error: next.code } : wire(next)), {
      status: failed ? next.status : 200,
      headers: { 'Content-Type': 'application/json' },
    });
  });
  vi.stubGlobal('fetch', double);
  return { sent, at, pending };
}

function wire(status: Record<string, unknown>) {
  return {
    request_id: REQUEST_ID,
    kind: 'set_step_assignment',
    ...status,
    deliveries: 1,
    expires_at: EXPIRES_AT,
    answered_at: null,
  };
}

async function advance(ms: number) {
  await act(() => vi.advanceTimersByTimeAsync(ms));
}

function statusLine(): HTMLElement {
  return screen.getByRole('status', { name: 'Request status' });
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

describe('RequestStatus', () => {
  it.each(['credential_not_pinned', 'some_future_reason'])('renders the refusal reason %s verbatim', async (reason) => {
    stubStatus([{ status: 'refused', reason }]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(0);

    const shown = screen.getByText(reason);
    expect(shown).toHaveTextContent(new RegExp(`^${reason}$`));
    expect(shown).toHaveClass('font-mono');
    expect(statusLine()).toHaveTextContent('Refused by the instance');
  });

  it('shows queued from the ticket before the first reply, with the expiry', () => {
    stubStatus([{ status: 'queued' }]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);

    expect(statusLine()).toHaveTextContent('Queued at the Hub');
    expect(statusLine().querySelector('time')).toHaveAttribute('datetime', new Date(EXPIRES_AT * 1000).toISOString());
  });

  it.each([
    [{ status: 'queued' }, 'Queued at the Hub', TONE_TEXT.cyan],
    [{ status: 'delivered' }, 'Delivered, waiting for the instance', TONE_TEXT.cyan],
    [{ status: 'accepted' }, 'Accepted by the instance', TONE_TEXT.emerald],
    [{ status: 'expired' }, 'Expired before the instance answered', TONE_TEXT.ruby],
  ])('labels %o as its own status', async (reply, label, tone) => {
    stubStatus([reply]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(0);

    expect(screen.getByText(label)).toHaveClass(tone);
    for (const other of LABELS.filter((each) => each !== label)) {
      expect(statusLine()).not.toHaveTextContent(other);
    }
  });

  it.each([
    { status: 'accepted' },
    { status: 'refused', reason: 'credential_not_pinned' },
    { status: 'expired' },
  ])('reads nothing more after %o', async (terminal) => {
    const { sent } = stubStatus([{ status: 'queued' }, { status: 'delivered' }, terminal]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(LONG_MS);

    expect(sent).toHaveLength(3);
  });

  it('stops polling on unmount', async () => {
    const { sent } = stubStatus([{ status: 'queued' }]);

    const { unmount } = render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(0);
    unmount();
    await advance(LONG_MS);

    expect(sent).toHaveLength(1);
  });

  it('ignores a reply that settles after unmount, and reads nothing more', async () => {
    const { sent } = stubStatus([{ status: 'queued' }]);

    const { unmount } = render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    unmount();
    await advance(LONG_MS);

    expect(sent).toHaveLength(1);
  });

  it('shows a 401 as a failed read, not as queued', async () => {
    const { sent } = stubStatus([{ status: 'queued' }, failure(401, 'unauthenticated')]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(LONG_MS);

    expect(statusLine()).toHaveTextContent('Could not read the request status');
    expect(statusLine()).not.toHaveTextContent('Queued at the Hub');
    expect(screen.getByRole('alert')).toHaveTextContent('unauthenticated');
    expect(sent).toHaveLength(2);
  });

  it('shows a persistent 409 as a failed read and reads nothing after the ticket expiry', async () => {
    const { at, pending } = stubStatus(Array.from({ length: 20 }, () => failure(409, 'scope_not_granted')));

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(LONG_MS);

    expect(statusLine()).toHaveTextContent('Could not read the request status');
    expect(statusLine()).not.toHaveTextContent('Queued at the Hub');
    expect(screen.getByRole('alert')).toHaveTextContent('scope_not_granted');
    expect(at[at.length - 1]).toBe(EXPIRES_AT);
    expect(at.every((second) => second <= EXPIRES_AT)).toBe(true);
    expect(pending.length).toBeGreaterThan(0);
  });

  it('clears a failed read once a later read answers', async () => {
    stubStatus([failure(500, 'store_failed'), { status: 'delivered' }]);

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(0);
    expect(statusLine()).toHaveTextContent('Could not read the request status');

    await advance(1000);
    expect(statusLine()).toHaveTextContent('Delivered, waiting for the instance');
    expect(statusLine()).not.toHaveTextContent('Could not read the request status');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each([
    { status: 'queued' },
    { status: 'delivered' },
  ])('shows expiry, not %o, when the Hub still answers it at the ticket expiry', async (live) => {
    const { at, pending } = stubStatus(Array.from({ length: 20 }, () => live));

    render(<RequestStatus instanceId={INSTANCE_ID} ticket={TICKET} />);
    await advance(LONG_MS);

    expect(statusLine()).toHaveTextContent('No answer from the instance before expiry');
    expect(statusLine()).toHaveTextContent('expired at');
    for (const label of LABELS) expect(statusLine()).not.toHaveTextContent(label);
    expect(at[at.length - 1]).toBe(EXPIRES_AT);
    expect(pending.length).toBeGreaterThan(0);
  });
});
