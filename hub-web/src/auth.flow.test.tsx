import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import App from './App';
import { setCsrfToken } from './lib/api';

const webauthn = vi.hoisted(() => ({
  createCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getAssertion: vi.fn<(options: unknown) => Promise<unknown>>(),
}));

/** The ceremonies are doubled and the error classes are not: `CeremonyError` tells them apart by `instanceof`. */
vi.mock('./lib/webauthn', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./lib/webauthn')>()),
  ...webauthn,
}));

type Exchange = { request: string; answer: Response };

type Sent = { request: string; csrf: string | null; body: unknown };

/**
 * Answers `script` in order and throws on any request that is not the next
 * one in it. `lib/api.ts` turns that throw into a `HubApiError` the shell
 * renders, so the flow also asserts that no alert is on the page.
 *
 * The Passkeys view shows nothing for a failed read of the list after an
 * action, so that one throw would pass unseen: `unscripted` holds every test
 * to it.
 */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      const next = pending.shift();
      if (next === undefined || next.request !== request) {
        unscripted.push(request);
        throw new Error(`fetch double was not told about ${request} here`);
      }
      sent.push({
        request,
        csrf: new Headers(init?.headers).get('X-CSRF-Token'),
        body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
      });
      return next.answer;
    }),
  );
  return { sent, pending };
}

function answer(request: string, body: unknown, status = 200): Exchange {
  return {
    request,
    answer: new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    }),
  };
}

function refuse(double: typeof webauthn.getAssertion, name: string): void {
  double.mockImplementation(async () => {
    throw new Error(`${name} double was not told to answer`);
  });
}

const CREATION_OPTIONS = { publicKey: { challenge: 'Y3JlYXRl' } };
const REQUEST_OPTIONS = { publicKey: { challenge: 'Z2V0' } };
const ATTESTATION = { id: 'cred-1', response: { attestationObject: 'YXR0' } };
const SIGN_IN_ASSERTION = { id: 'cred-1', response: { signature: 'c2ln' } };
const REVOKING_ASSERTION = {
  credential_id: 'cred-4',
  authenticator_data: 'YXV0aA',
  client_data_json: 'Y2xpZW50',
  signature: 'c2ln',
};

const PAIRING_REQUEST = 'eyJyZXF1ZXN0IjoiMSJ9';
const PAIRING_LINK = `#/pair/${PAIRING_REQUEST}`;
const PAIRING_PROMPT = {
  pairing_id: 'pairing-1',
  fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
  instance_name: 'workstation',
  os: 'linux',
  arch: 'x86_64',
  app_version: '1.4.2',
  scopes: ['read', 'gates'],
  expires_at: 1_700_000_300,
};

function wirePasskey(credential_id: string, label: string, status: string, current: boolean) {
  return { credential_id, label, status, created_at: 1_700_000_000, current };
}

let unscripted: string[] = [];

beforeEach(() => {
  unscripted = [];
  refuse(webauthn.createCredential, 'createCredential');
  refuse(webauthn.getCredential, 'getCredential');
  refuse(webauthn.getAssertion, 'getAssertion');
});

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  window.location.hash = '';
  // Soft, because a hook that throws stops the hooks after it: the cleanup in
  // `src/test/setup.ts` would not run, and the next test would fail as well.
  expect.soft(unscripted).toEqual([]);
});

function shownView(): string | null {
  return screen.getByRole('heading', { level: 1 }).textContent;
}

describe('hub-web, from no account to signed back in', () => {
  it('signs up with a first passkey, signs out, and signs in again', async () => {
    const { sent, pending } = stubFetch([
      answer('GET /v1/session', { error: 'unauthenticated' }, 401),
      answer('POST /v1/register/start', { ceremony_id: 'cer-register', options: CREATION_OPTIONS }),
      answer('POST /v1/register/finish', { csrf_token: 'csrf-from-register', credential_id: 'cred-1' }),
      answer('GET /v1/runs?status=running', { runs: [] }),
      answer('GET /v1/fleet', { instances: [] }),
      answer('POST /v1/session/end', {}),
      answer('POST /v1/session/start', { ceremony_id: 'cer-sign-in', options: REQUEST_OPTIONS }),
      answer('POST /v1/session/finish', { csrf_token: 'csrf-from-sign-in', credential_id: 'cred-1' }),
      answer('GET /v1/runs?status=running', { runs: [] }),
      answer('GET /v1/fleet', { instances: [] }),
    ]);
    webauthn.createCredential.mockResolvedValueOnce(ATTESTATION);
    webauthn.getCredential.mockResolvedValueOnce(SIGN_IN_ASSERTION);
    render(<App />);

    await userEvent.click(await screen.findByRole('link', { name: 'Sign up' }));
    await userEvent.type(await screen.findByLabelText('Passkey name'), 'laptop');
    await userEvent.click(screen.getByRole('button', { name: 'Create passkey' }));

    const signOutControl = await screen.findByRole('button', { name: 'Sign out' });
    expect(screen.getByRole('navigation')).toBeInTheDocument();
    expect(shownView()).toBe('Fleet');
    await screen.findByText(/no instances are paired/i);
    expect(sent.slice(1)).toEqual([
      { request: 'POST /v1/register/start', csrf: null, body: { label: 'laptop' } },
      {
        request: 'POST /v1/register/finish',
        csrf: null,
        body: { ceremony_id: 'cer-register', credential: ATTESTATION },
      },
      { request: 'GET /v1/runs?status=running', csrf: null, body: undefined },
      { request: 'GET /v1/fleet', csrf: null, body: undefined },
    ]);
    expect(webauthn.createCredential.mock.calls).toEqual([[CREATION_OPTIONS]]);

    await userEvent.click(signOutControl);

    const signInControl = await screen.findByRole('button', { name: 'Sign in with a passkey' });
    expect(shownView()).toBe('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(sent.slice(5)).toEqual([
      { request: 'POST /v1/session/end', csrf: 'csrf-from-register', body: {} },
    ]);

    await userEvent.click(signInControl);

    await screen.findByRole('button', { name: 'Sign out' });
    expect(shownView()).toBe('Fleet');
    await screen.findByText(/no instances are paired/i);
    expect(sent.slice(6)).toEqual([
      { request: 'POST /v1/session/start', csrf: null, body: {} },
      {
        request: 'POST /v1/session/finish',
        csrf: null,
        body: { ceremony_id: 'cer-sign-in', credential: SIGN_IN_ASSERTION },
      },
      { request: 'GET /v1/runs?status=running', csrf: null, body: undefined },
      { request: 'GET /v1/fleet', csrf: null, body: undefined },
    ]);
    expect(webauthn.getCredential.mock.calls).toEqual([[REQUEST_OPTIONS]]);
    expect(webauthn.getAssertion).not.toHaveBeenCalled();
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('hub-web, opened signed out on a pairing link', () => {
  it('shows the sign-in view, and signing in lands on the approval page of that link', async () => {
    const { sent, pending } = stubFetch([
      answer('GET /v1/session', { error: 'unauthenticated' }, 401),
      answer('POST /v1/session/start', { ceremony_id: 'cer-sign-in', options: REQUEST_OPTIONS }),
      answer('POST /v1/session/finish', { csrf_token: 'csrf-from-sign-in', credential_id: 'cred-1' }),
      answer('POST /v1/pairing/start', PAIRING_PROMPT),
      answer('GET /v1/fleet', { instances: [] }),
    ]);
    webauthn.getCredential.mockResolvedValueOnce(SIGN_IN_ASSERTION);
    window.location.hash = PAIRING_LINK;
    render(<App />);

    const signInControl = await screen.findByRole('button', { name: 'Sign in with a passkey' });
    expect(shownView()).toBe('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(window.location.hash).toBe(PAIRING_LINK);
    // Signed out, the Hub is not asked about the request the link carries.
    expect(sent).toEqual([{ request: 'GET /v1/session', csrf: null, body: undefined }]);

    await userEvent.click(signInControl);

    await screen.findByRole('button', { name: 'Approve' });
    expect(shownView()).toBe('Pair instance');
    expect(window.location.hash).toBe(PAIRING_LINK);
    const key = screen.getByRole('region', { name: 'Device key fingerprint' });
    expect(within(key).getByText(PAIRING_PROMPT.fingerprint)).toBeInTheDocument();
    expect(screen.getByText(PAIRING_PROMPT.instance_name)).toBeInTheDocument();
    expect(screen.getAllByRole('checkbox')).toHaveLength(2);
    expect(screen.getByRole('checkbox', { name: 'read' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: 'gates' })).toBeChecked();
    expect(sent.slice(1)).toEqual([
      { request: 'POST /v1/session/start', csrf: null, body: {} },
      {
        request: 'POST /v1/session/finish',
        csrf: null,
        body: { ceremony_id: 'cer-sign-in', credential: SIGN_IN_ASSERTION },
      },
      { request: 'POST /v1/pairing/start', csrf: 'csrf-from-sign-in', body: { request: PAIRING_REQUEST } },
      { request: 'GET /v1/fleet', csrf: null, body: undefined },
    ]);
    expect(webauthn.getCredential.mock.calls).toEqual([[REQUEST_OPTIONS]]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('hub-web, when the session ends under it', () => {
  it('shows the sign-in view once the passkey it signed in with is revoked, and signs in afresh', async () => {
    const { sent, pending } = stubFetch([
      answer('GET /v1/session', { csrf_token: 'csrf-laptop', credential_id: 'cred-1' }),
      answer('GET /v1/passkeys', {
        passkeys: [
          wirePasskey('cred-1', 'laptop', 'trusted', true),
          wirePasskey('cred-4', 'tablet', 'trusted', false),
        ],
      }),
      answer('GET /v1/fleet', { instances: [] }),
      answer('POST /v1/passkeys/revoke/start', { ceremony_id: 'cer-revoke', options: REQUEST_OPTIONS }),
      answer('POST /v1/passkeys/revoke/finish', {
        passkey: wirePasskey('cred-1', 'laptop', 'revoked', true),
      }),
      answer('GET /v1/passkeys', { error: 'unauthenticated' }, 401),
      answer('POST /v1/session/start', { ceremony_id: 'cer-sign-in', options: REQUEST_OPTIONS }),
      answer('POST /v1/session/finish', { csrf_token: 'csrf-tablet', credential_id: 'cred-4' }),
      answer('GET /v1/passkeys', {
        passkeys: [
          wirePasskey('cred-1', 'laptop', 'revoked', false),
          wirePasskey('cred-4', 'tablet', 'trusted', true),
        ],
      }),
      answer('GET /v1/fleet', { instances: [] }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(REVOKING_ASSERTION);
    webauthn.getCredential.mockResolvedValueOnce(SIGN_IN_ASSERTION);
    window.location.hash = '#/passkeys';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Revoke laptop' }));

    const signInControl = await screen.findByRole('button', { name: 'Sign in with a passkey' });
    expect(shownView()).toBe('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(sent.slice(3)).toEqual([
      { request: 'POST /v1/passkeys/revoke/start', csrf: 'csrf-laptop', body: { credential_id: 'cred-1' } },
      {
        request: 'POST /v1/passkeys/revoke/finish',
        csrf: 'csrf-laptop',
        body: { ceremony_id: 'cer-revoke', assertion: REVOKING_ASSERTION },
      },
      // The read the view makes after every action, sent here by a session
      // the revocation ended. Its `401` is the answer of a signed-out read.
      { request: 'GET /v1/passkeys', csrf: null, body: undefined },
    ]);

    await userEvent.click(signInControl);

    // The hash never left `#/passkeys`, so that is where signing in lands.
    await screen.findByRole('button', { name: 'Revoke tablet' });
    expect(shownView()).toBe('Passkeys');
    expect(screen.getByRole('navigation')).toBeInTheDocument();
    expect(sent.slice(6)).toEqual([
      { request: 'POST /v1/session/start', csrf: null, body: {} },
      {
        request: 'POST /v1/session/finish',
        csrf: null,
        body: { ceremony_id: 'cer-sign-in', credential: SIGN_IN_ASSERTION },
      },
      { request: 'GET /v1/passkeys', csrf: null, body: undefined },
      { request: 'GET /v1/fleet', csrf: null, body: undefined },
    ]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the sign-in view when the list read after a failed action is answered 401', async () => {
    const { sent, pending } = stubFetch([
      answer('GET /v1/session', { csrf_token: 'csrf-laptop', credential_id: 'cred-1' }),
      answer('GET /v1/passkeys', {
        passkeys: [
          wirePasskey('cred-1', 'laptop', 'trusted', true),
          wirePasskey('cred-2', 'phone', 'pending', false),
        ],
      }),
      answer('GET /v1/fleet', { instances: [] }),
      answer('POST /v1/passkeys/endorse/start', { error: 'internal' }, 500),
      answer('GET /v1/passkeys', { error: 'unauthenticated' }, 401),
    ]);
    window.location.hash = '#/passkeys';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Endorse phone' }));

    await screen.findByRole('button', { name: 'Sign in with a passkey' });
    expect(shownView()).toBe('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(sent.slice(3)).toEqual([
      { request: 'POST /v1/passkeys/endorse/start', csrf: 'csrf-laptop', body: { credential_id: 'cred-2' } },
      { request: 'GET /v1/passkeys', csrf: null, body: undefined },
    ]);
    expect(pending).toEqual([]);
  });

  it('still shows a refused sign-in on the sign-in view', async () => {
    const { pending } = stubFetch([
      answer('GET /v1/session', { error: 'unauthenticated' }, 401),
      answer('POST /v1/session/start', { ceremony_id: 'cer-sign-in', options: REQUEST_OPTIONS }),
      answer('POST /v1/session/finish', { error: 'unauthenticated' }, 401),
    ]);
    webauthn.getCredential.mockResolvedValueOnce(SIGN_IN_ASSERTION);
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Sign in with a passkey' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^unauthenticated$/);
    expect(shownView()).toBe('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.getByRole('button', { name: 'Sign in with a passkey' })).toBeEnabled();
    expect(pending).toEqual([]);
  });
});
