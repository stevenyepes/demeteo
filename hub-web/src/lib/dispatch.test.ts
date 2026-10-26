import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import {
  type DispatchRequest,
  dispatchErrorSentence,
  dispatchFeature,
  getRequestStatus,
  refusalSentence,
} from './dispatch';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const REQUEST_ID = 'q8Hn3_kZ-1';
const RUNS_PATH = `/v1/instances/${INSTANCE_ID}/runs`;
const STATUS_PATH = `/v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;

const BODY: DispatchRequest = {
  project_id: 'p-1',
  workflow_id: 'wf-1',
  title: 'Add a login form',
  description: 'With a remember-me box',
  step_overrides: [{ step_id: 's-implement', agent_kind: 'codex' }],
  max_budget_cents: 500,
  attachment_ids: ['att-1'],
};

const WIRE_STATUS = {
  request_id: REQUEST_ID,
  kind: 'start_feature',
  status: 'queued',
  deliveries: 0,
  expires_at: 1_700_000_600,
  answered_at: null,
};

/** Every request the double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Answers only `method` on `path`; anything else throws and is recorded. */
function stubFetch(method: string, path: string, answer: Response) {
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const sent = init?.method ?? 'GET';
    if (sent !== method || input !== path) {
      unscripted.push(`${sent} ${String(input)}`);
      throw new Error(`fetch double was not told about ${sent} ${String(input)}`);
    }
    return answer;
  });
  vi.stubGlobal('fetch', double);
  return double;
}

function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

async function rejectionOf(pending: Promise<unknown>): Promise<HubApiError> {
  const error: unknown = await pending.then(
    () => null,
    (reason: unknown) => reason,
  );
  if (!(error instanceof HubApiError)) throw new Error(`expected a HubApiError, got ${String(error)}`);
  return error;
}

afterEach(() => {
  expect(unscripted).toEqual([]);
  unscripted = [];
  vi.unstubAllGlobals();
  setCsrfToken(null);
});

describe('dispatchFeature', () => {
  it('maps a 202 to Enqueued', async () => {
    stubFetch('POST', RUNS_PATH, json({ request_id: REQUEST_ID, expires_at: 1_700_000_600 }, 202));

    await expect(dispatchFeature(INSTANCE_ID, BODY)).resolves.toEqual({
      requestId: REQUEST_ID,
      expiresAt: 1_700_000_600,
    });
  });

  it('posts the body as it stands, with the CSRF token', async () => {
    setCsrfToken('csrf-1');
    const fetchDouble = stubFetch(
      'POST',
      RUNS_PATH,
      json({ request_id: REQUEST_ID, expires_at: 1_700_000_600 }, 202),
    );

    await dispatchFeature(INSTANCE_ID, BODY);

    const init = fetchDouble.mock.calls[0]?.[1];
    expect(JSON.parse(String(init?.body))).toEqual(BODY);
    expect(new Headers(init?.headers).get('X-CSRF-Token')).toBe('csrf-1');
  });

  it('rejects a 202 missing its expiry', async () => {
    stubFetch('POST', RUNS_PATH, json({ request_id: REQUEST_ID }, 202));

    const error = await rejectionOf(dispatchFeature(INSTANCE_ID, BODY));

    expect(error.status).toBe(202);
  });

  it('refuses a non-UUID instance id without a request', async () => {
    const fetchDouble = stubFetch('POST', RUNS_PATH, json({}, 202));

    const error = await rejectionOf(dispatchFeature('../fleet', BODY));

    expect(error.status).toBe(404);
    expect(error.code).toBe('not_found');
    expect(fetchDouble).not.toHaveBeenCalled();
  });
});

describe('getRequestStatus', () => {
  it.each(['queued', 'delivered', 'accepted', 'expired'])('accepts %s', async (status) => {
    stubFetch('GET', STATUS_PATH, json({ ...WIRE_STATUS, status }, 200));

    await expect(getRequestStatus(INSTANCE_ID, REQUEST_ID)).resolves.toEqual({
      requestId: REQUEST_ID,
      kind: 'start_feature',
      status,
      deliveries: 0,
      expiresAt: 1_700_000_600,
      answeredAt: null,
    });
  });

  it('accepts refused with its reason and answer time', async () => {
    stubFetch(
      'GET',
      STATUS_PATH,
      json(
        { ...WIRE_STATUS, status: 'refused', reason: 'over_run_ceiling', deliveries: 1, answered_at: 1_700_000_100 },
        200,
      ),
    );

    await expect(getRequestStatus(INSTANCE_ID, REQUEST_ID)).resolves.toEqual({
      requestId: REQUEST_ID,
      kind: 'start_feature',
      status: 'refused',
      reason: 'over_run_ceiling',
      deliveries: 1,
      expiresAt: 1_700_000_600,
      answeredAt: 1_700_000_100,
    });
  });

  it('accepts a reason it has no sentence for', async () => {
    stubFetch('GET', STATUS_PATH, json({ ...WIRE_STATUS, status: 'refused', reason: 'brand_new_reason' }, 200));

    await expect(getRequestStatus(INSTANCE_ID, REQUEST_ID)).resolves.toMatchObject({
      status: 'refused',
      reason: 'brand_new_reason',
    });
  });

  it('rejects an unknown status', async () => {
    stubFetch('GET', STATUS_PATH, json({ ...WIRE_STATUS, status: 'sent' }, 200));

    const error = await rejectionOf(getRequestStatus(INSTANCE_ID, REQUEST_ID));

    expect(error.status).toBe(200);
  });

  it('rejects refused without a reason', async () => {
    stubFetch('GET', STATUS_PATH, json({ ...WIRE_STATUS, status: 'refused' }, 200));

    const error = await rejectionOf(getRequestStatus(INSTANCE_ID, REQUEST_ID));

    expect(error.status).toBe(200);
  });

  it('rejects an answered_at that is neither a number nor null', async () => {
    stubFetch('GET', STATUS_PATH, json({ ...WIRE_STATUS, answered_at: 'soon' }, 200));

    const error = await rejectionOf(getRequestStatus(INSTANCE_ID, REQUEST_ID));

    expect(error.status).toBe(200);
  });

  it('encodes the request id as one path segment', async () => {
    const fetchDouble = stubFetch('GET', `/v1/instances/${INSTANCE_ID}/requests/a%2F..%2Fb`, json(WIRE_STATUS, 200));

    await getRequestStatus(INSTANCE_ID, 'a/../b');

    expect(fetchDouble).toHaveBeenCalledOnce();
  });

  it('refuses a non-UUID instance id without a request', async () => {
    const fetchDouble = stubFetch('GET', STATUS_PATH, json(WIRE_STATUS, 200));

    const error = await rejectionOf(getRequestStatus('../fleet', REQUEST_ID));

    expect(error.status).toBe(404);
    expect(error.code).toBe('not_found');
    expect(fetchDouble).not.toHaveBeenCalled();
  });
});

describe('refusalSentence', () => {
  it.each([
    'scope_off',
    'over_run_ceiling',
    'over_rolling_ceiling',
    'unknown_project',
    'unknown_workflow',
    'harness_not_registered',
    'attachment_mismatch',
    'malformed_body',
    'expired',
  ])('has a sentence for %s', (reason) => {
    expect(refusalSentence(reason)).toEqual(expect.any(String));
  });

  it('says what over_run_ceiling and scope_off mean', () => {
    expect(refusalSentence('over_run_ceiling')).toMatch(/per-run ceiling/);
    expect(refusalSentence('scope_off')).toMatch(/spend is off/i);
  });

  it.each(['brand_new_reason', 'constructor', ''])('has none for %j', (reason) => {
    expect(refusalSentence(reason)).toBeNull();
  });
});

describe('dispatchErrorSentence', () => {
  function errorWith(code: string | null, scope: string | null = null): HubApiError {
    return new HubApiError('refused', RUNS_PATH, 409, code, scope);
  }

  it('reads a withheld spend scope as spend being off', () => {
    expect(dispatchErrorSentence(errorWith('scope_not_granted', 'spend'))).toMatch(/spend is off on this instance/i);
  });

  it('has no sentence for a withheld scope other than spend', () => {
    expect(dispatchErrorSentence(errorWith('scope_not_granted', 'read'))).toBeNull();
  });

  it.each([
    'attachment_unavailable',
    'attachment_too_large',
    'invalid_request',
    'key_service_unavailable',
  ])('has a sentence for %s', (code) => {
    expect(dispatchErrorSentence(errorWith(code))).toEqual(expect.any(String));
  });

  it.each([null, 'not_found', 'constructor'])('has none for %j', (code) => {
    expect(dispatchErrorSentence(errorWith(code))).toBeNull();
  });
});
