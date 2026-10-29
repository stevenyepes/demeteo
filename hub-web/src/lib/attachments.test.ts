import { afterEach, describe, expect, it, vi } from 'vitest';

import { HubApiError, setCsrfToken } from './api';
import { uploadAttachment } from './attachments';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const UPLOAD_PATH = `/v1/instances/${INSTANCE_ID}/attachments`;

const WIRE_STAGED = {
  attachment_id: 'att-1',
  size: 5,
  sha256: '2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824',
  expires_at: 1_700_000_900,
};

/** Every request the double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Answers only a `POST` to the upload path; anything else throws and is recorded. */
function stubFetch(answer: Response) {
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    if (method !== 'POST' || input !== UPLOAD_PATH) {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)}`);
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

describe('uploadAttachment', () => {
  it('maps a 201 to a StagedAttachment', async () => {
    stubFetch(json(WIRE_STAGED, 201));

    await expect(uploadAttachment(INSTANCE_ID, new Blob(['hello']))).resolves.toEqual({
      attachmentId: 'att-1',
      size: 5,
      sha256: WIRE_STAGED.sha256,
      expiresAt: 1_700_000_900,
    });
  });

  it('sends the raw bytes as the body, with the CSRF token', async () => {
    setCsrfToken('csrf-1');
    const fetchDouble = stubFetch(json(WIRE_STAGED, 201));
    const file = new File(['hello'], 'notes.txt', { type: 'text/plain' });

    await uploadAttachment(INSTANCE_ID, file);

    const init = fetchDouble.mock.calls[0]?.[1];
    expect(init?.body).toBe(file);
    expect(new Headers(init?.headers).get('X-CSRF-Token')).toBe('csrf-1');
  });

  it('rejects a 413 with code attachment_too_large', async () => {
    stubFetch(json({ error: 'attachment_too_large' }, 413));

    const error = await rejectionOf(uploadAttachment(INSTANCE_ID, new Blob(['hello'])));

    expect(error.status).toBe(413);
    expect(error.code).toBe('attachment_too_large');
  });

  it('rejects a withheld spend scope carrying the scope', async () => {
    stubFetch(json({ error: 'scope_not_granted', scope: 'spend' }, 409));

    const error = await rejectionOf(uploadAttachment(INSTANCE_ID, new Blob(['hello'])));

    expect(error.code).toBe('scope_not_granted');
    expect(error.scope).toBe('spend');
  });

  it('rejects a 201 missing its sha256', async () => {
    const { sha256: _, ...withoutDigest } = WIRE_STAGED;
    stubFetch(json(withoutDigest, 201));

    const error = await rejectionOf(uploadAttachment(INSTANCE_ID, new Blob(['hello'])));

    expect(error.status).toBe(201);
  });

  it('refuses a non-UUID instance id without a request', async () => {
    const fetchDouble = stubFetch(json(WIRE_STAGED, 201));

    const error = await rejectionOf(uploadAttachment('../fleet', new Blob(['hello'])));

    expect(error.status).toBe(404);
    expect(error.code).toBe('not_found');
    expect(fetchDouble).not.toHaveBeenCalled();
  });
});
