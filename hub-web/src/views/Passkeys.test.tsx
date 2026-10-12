import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { WebAuthnCancelledError, WebAuthnUnavailableError } from '../lib/webauthn';
import { Passkeys } from './Passkeys';

const webauthn = vi.hoisted(() => ({
  createCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getCredential: vi.fn<(options: unknown) => Promise<unknown>>(),
  getAssertion: vi.fn<(options: unknown) => Promise<unknown>>(),
}));

/** The ceremonies are doubled and the error classes are not: the view tells them apart by `instanceof`. */
vi.mock('../lib/webauthn', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/webauthn')>()),
  ...webauthn,
}));

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response | Promise<Response> };

type Sent = { path: string; body: unknown };

/**
 * Answers `script` in order and throws on any request that is not the next
 * one in it. `lib/api.ts` turns that throw into a `HubApiError`, which the
 * view renders, so every test that expects no failure also asserts that no
 * alert is on the page.
 *
 * The read the view makes after an action is the exception: its failure is
 * shown nowhere, so the throw alone would pass unseen. `unscripted` is what
 * every test is held to instead.
 */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  const double = vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
    const method = init?.method ?? 'GET';
    const next = pending.shift();
    if (next === undefined || next.path !== input || next.method !== method) {
      unscripted.push(`${method} ${String(input)}`);
      throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
    }
    sent.push({
      path: next.path,
      body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
    });
    return next.answer;
  });
  vi.stubGlobal('fetch', double);
  return { sent, pending };
}

function post(path: string, body: unknown, status = 200): Exchange {
  return { method: 'POST', path, answer: json(body, status) };
}

function listing(...passkeys: WirePasskey[]): Exchange {
  return { method: 'GET', path: '/v1/passkeys', answer: json({ passkeys }, 200) };
}

/** A list read that gets no response, failed by the returned function once the test is ready to look. */
function unansweredListing(): { exchange: Exchange; fail: () => Promise<void> } {
  let reject: (reason: Error) => void = () => {};
  const answer = new Promise<Response>((_, rejected) => {
    reject = rejected;
  });
  return {
    exchange: { method: 'GET', path: '/v1/passkeys', answer },
    // Everything the view does with the failure is microtasks behind the
    // rejection, so one task later it has done all it is going to.
    fail: () =>
      act(async () => {
        reject(new Error('offline'));
        await new Promise((resolve) => setTimeout(resolve, 0));
      }),
  };
}

function json(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function refuse(double: typeof webauthn.getAssertion, name: string): void {
  double.mockImplementation(async () => {
    throw new Error(`${name} double was not told to answer`);
  });
}

type WirePasskey = {
  credential_id: string;
  label: string;
  status: 'pending' | 'trusted' | 'revoked';
  created_at: number;
  current: boolean;
};

function wire(
  credential_id: string,
  label: string,
  status: WirePasskey['status'],
  current = false,
): WirePasskey {
  return { credential_id, label, status, created_at: 1_700_000_000, current };
}

const LAPTOP = wire('cred-1', 'laptop', 'trusted', true);
const PHONE = wire('cred-2', 'phone', 'pending');
const OLD_KEY = wire('cred-3', 'old key', 'revoked');
const TABLET = wire('cred-4', 'tablet', 'trusted');
const DESK_KEY = wire('cred-5', 'desk key', 'pending');

const EXPIRED_CEREMONY_HINT = /no longer holding this request/;

const CREATION_OPTIONS = { publicKey: { challenge: 'Y3JlYXRl' } };
const REQUEST_OPTIONS = { publicKey: { challenge: 'Z2V0' } };
const ATTESTATION = { id: 'cred-2', response: { attestationObject: 'YXR0' } };
const ASSERTION = {
  credential_id: 'cred-1',
  authenticator_data: 'YXV0aA',
  client_data_json: 'Y2xpZW50',
  signature: 'c2ln',
};

async function rowOf(label: string): Promise<HTMLElement> {
  const row = (await screen.findByText(label)).closest('li');
  if (row === null) throw new Error(`no row holds ${label}`);
  return row;
}

async function statusOf(label: string): Promise<string | null> {
  return within(await rowOf(label)).getByText(/^(Trusted|Pending|Revoked)$/).textContent;
}

async function controlsOf(label: string): Promise<{ name: string | null; enabled: boolean }[]> {
  return within(await rowOf(label))
    .queryAllByRole('button')
    .map((button) => ({
      name: button.getAttribute('aria-label'),
      enabled: !(button as HTMLButtonElement).disabled,
    }));
}

async function click(name: string): Promise<void> {
  await userEvent.click(await screen.findByRole('button', { name }));
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
  // Soft, because a hook that throws stops the hooks after it: the cleanup in
  // `src/test/setup.ts` would not run, and the next test would fail as well.
  expect.soft(unscripted).toEqual([]);
});

describe('Passkeys', () => {
  it('lists each passkey with its status, in the order sent, and marks the one signed in with', async () => {
    const { pending } = stubFetch([listing(LAPTOP, PHONE, OLD_KEY)]);
    render(<Passkeys />);

    expect(await statusOf('laptop')).toBe('Trusted');
    expect(await statusOf('phone')).toBe('Pending');
    expect(await statusOf('old key')).toBe('Revoked');
    expect(screen.getAllByRole('listitem').map((row) => row.textContent)).toEqual([
      expect.stringMatching(/^laptop/),
      expect.stringMatching(/^phone/),
      expect.stringMatching(/^old key/),
    ]);
    expect(within(await rowOf('laptop')).getByText('This session')).toBeInTheDocument();
    expect(screen.getAllByText('This session')).toHaveLength(1);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('offers endorse on a pending row only, and revoke on every row that is not revoked', async () => {
    stubFetch([listing(LAPTOP, TABLET, PHONE, OLD_KEY)]);
    render(<Passkeys />);

    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: true }]);
    expect(await controlsOf('tablet')).toEqual([{ name: 'Revoke tablet', enabled: true }]);
    expect(await controlsOf('phone')).toEqual([
      { name: 'Endorse phone', enabled: true },
      { name: 'Revoke phone', enabled: true },
    ]);
    expect(await controlsOf('old key')).toEqual([]);
  });

  it('adds a passkey under the trimmed label and leaves its row Pending', async () => {
    const { sent, pending } = stubFetch([
      listing(LAPTOP),
      post('/v1/passkeys/add/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      post('/v1/passkeys/add/finish', { passkey: PHONE }),
      listing(LAPTOP, PHONE),
    ]);
    webauthn.createCredential.mockResolvedValueOnce(ATTESTATION);
    render(<Passkeys />);
    await rowOf('laptop');
    expect(screen.queryByText(/already trusted must approve/)).toBeNull();

    await userEvent.type(screen.getByLabelText('Passkey name'), '  phone  ');
    await click('Add passkey');

    expect(await statusOf('phone')).toBe('Pending');
    expect(await statusOf('laptop')).toBe('Trusted');
    expect(screen.getByText(/already trusted must approve/)).toBeInTheDocument();
    expect(sent.slice(1)).toEqual([
      { path: '/v1/passkeys/add/start', body: { label: 'phone' } },
      { path: '/v1/passkeys/add/finish', body: { ceremony_id: 'cer-1', credential: ATTESTATION } },
      { path: '/v1/passkeys', body: undefined },
    ]);
    expect(webauthn.createCredential.mock.calls).toEqual([[CREATION_OPTIONS]]);
    expect(webauthn.getAssertion).not.toHaveBeenCalled();
    expect(screen.getByLabelText('Passkey name')).toHaveValue('');
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each([
    { typed: '', why: 'an empty label' },
    { typed: '   ', why: 'a label of spaces' },
    { typed: 'x'.repeat(65), why: 'a label of 65 characters' },
  ])('cannot add a passkey with $why', async ({ typed }) => {
    const { sent } = stubFetch([listing(LAPTOP)]);
    render(<Passkeys />);
    await rowOf('laptop');

    if (typed !== '') await userEvent.type(screen.getByLabelText('Passkey name'), typed);

    expect(screen.getByRole('button', { name: 'Add passkey' })).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Passkey name'), '{Enter}');
    expect(sent).toHaveLength(1);
  });

  it('endorses by posting the assertion to endorse/finish, and the row is Trusted only once the Hub answers', async () => {
    let answer: (response: Response) => void = () => {};
    const { sent, pending } = stubFetch([
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      {
        method: 'POST',
        path: '/v1/passkeys/endorse/finish',
        answer: new Promise<Response>((resolve) => {
          answer = resolve;
        }),
      },
      listing(LAPTOP, { ...PHONE, status: 'trusted' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Endorse phone');
    await waitFor(() => expect(sent).toHaveLength(3));

    expect(sent.slice(1)).toEqual([
      { path: '/v1/passkeys/endorse/start', body: { credential_id: 'cred-2' } },
      { path: '/v1/passkeys/endorse/finish', body: { ceremony_id: 'cer-2', assertion: ASSERTION } },
    ]);
    expect(webauthn.getAssertion.mock.calls).toEqual([[REQUEST_OPTIONS]]);
    expect(await statusOf('phone')).toBe('Pending');
    expect(screen.getByRole('button', { name: 'Endorse phone' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Revoke phone' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Add passkey' })).toBeDisabled();

    answer(json({ passkey: { ...PHONE, status: 'trusted' } }, 200));

    await waitFor(async () => expect(await statusOf('phone')).toBe('Trusted'));
    expect(await controlsOf('phone')).toEqual([{ name: 'Revoke phone', enabled: true }]);
    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: true }]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the status the Hub answered an endorsement with, not the one asked for', async () => {
    stubFetch([
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { passkey: PHONE }),
      listing(LAPTOP, PHONE),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Endorse phone');

    await waitFor(() => expect(screen.getByRole('button', { name: 'Endorse phone' })).toBeEnabled());
    expect(await statusOf('phone')).toBe('Pending');
  });

  it('revokes by posting the assertion to revoke/finish, and the row becomes Revoked', async () => {
    const { sent, pending } = stubFetch([
      listing(LAPTOP, TABLET),
      post('/v1/passkeys/revoke/start', { ceremony_id: 'cer-3', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/revoke/finish', { passkey: { ...TABLET, status: 'revoked' } }),
      listing(LAPTOP, { ...TABLET, status: 'revoked' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Revoke tablet');

    await waitFor(async () => expect(await statusOf('tablet')).toBe('Revoked'));
    expect(sent.slice(1)).toEqual([
      { path: '/v1/passkeys/revoke/start', body: { credential_id: 'cred-4' } },
      { path: '/v1/passkeys/revoke/finish', body: { ceremony_id: 'cer-3', assertion: ASSERTION } },
      { path: '/v1/passkeys', body: undefined },
    ]);
    expect(webauthn.getAssertion.mock.calls).toEqual([[REQUEST_OPTIONS]]);
    expect(await controlsOf('tablet')).toEqual([]);
    expect(await statusOf('laptop')).toBe('Trusted');
    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: false }]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('has no enabled revoke control for the only trusted passkey', async () => {
    const { sent } = stubFetch([listing(LAPTOP, PHONE, OLD_KEY)]);
    render(<Passkeys />);

    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: false }]);
    expect(await controlsOf('phone')).toContainEqual({ name: 'Revoke phone', enabled: true });

    await userEvent.click(screen.getByRole('button', { name: 'Revoke laptop' }));
    expect(sent).toHaveLength(1);
  });

  it.each([
    {
      code: 'self_endorsement',
      listed: [LAPTOP, PHONE],
      control: 'Endorse phone',
      script: [
        post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
        post('/v1/passkeys/endorse/finish', { error: 'self_endorsement' }, 403),
      ],
      asserts: true,
      row: 'phone',
      stays: 'Pending',
    },
    {
      code: 'last_passkey',
      listed: [LAPTOP, TABLET],
      control: 'Revoke tablet',
      script: [post('/v1/passkeys/revoke/start', { error: 'last_passkey' }, 409)],
      asserts: false,
      row: 'tablet',
      stays: 'Trusted',
    },
    {
      code: 'self_revocation',
      listed: [LAPTOP, TABLET],
      control: 'Revoke tablet',
      script: [
        post('/v1/passkeys/revoke/start', { ceremony_id: 'cer-3', options: REQUEST_OPTIONS }),
        post('/v1/passkeys/revoke/finish', { error: 'self_revocation' }, 403),
      ],
      asserts: true,
      row: 'tablet',
      stays: 'Trusted',
    },
  ])('renders a $code refusal as an error and leaves the row $stays', async (refusal) => {
    const { pending } = stubFetch([
      listing(...refusal.listed),
      ...refusal.script,
      listing(...refusal.listed),
    ]);
    if (refusal.asserts) webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click(refusal.control);

    expect(await screen.findByRole('alert')).toHaveTextContent(new RegExp(`^${refusal.code}$`));
    expect(screen.queryByText(EXPIRED_CEREMONY_HINT)).toBeNull();
    expect(await statusOf(refusal.row)).toBe(refusal.stays);
    expect(screen.getByRole('button', { name: refusal.control })).toBeEnabled();
    expect(pending).toEqual([]);
  });

  it('renders WebAuthn being unavailable as an error and adds no row', async () => {
    const { pending } = stubFetch([
      listing(LAPTOP),
      post('/v1/passkeys/add/start', { ceremony_id: 'cer-1', options: CREATION_OPTIONS }),
      listing(LAPTOP),
    ]);
    webauthn.createCredential.mockRejectedValueOnce(new WebAuthnUnavailableError());
    render(<Passkeys />);
    await rowOf('laptop');

    await userEvent.type(screen.getByLabelText('Passkey name'), 'phone');
    await click('Add passkey');

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /^This browser does not offer passkeys here\./,
    );
    expect(screen.getAllByRole('listitem')).toHaveLength(1);
    expect(screen.getByLabelText('Passkey name')).toHaveValue('phone');
    expect(pending).toEqual([]);
  });

  it('renders a cancelled ceremony as an error, and the next attempt clears it', async () => {
    const { pending } = stubFetch([
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-4', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { passkey: { ...PHONE, status: 'trusted' } }),
      listing(LAPTOP, { ...PHONE, status: 'trusted' }),
    ]);
    webauthn.getAssertion
      .mockRejectedValueOnce(new WebAuthnCancelledError())
      .mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Endorse phone');

    expect(await screen.findByRole('alert')).toHaveTextContent(
      /^The passkey prompt was cancelled, timed out, or the authenticator refused\./,
    );
    expect(await statusOf('phone')).toBe('Pending');

    await click('Endorse phone');

    await waitFor(async () => expect(await statusOf('phone')).toBe('Trusted'));
    expect(screen.queryByRole('alert')).toBeNull();
    expect(pending).toEqual([]);
  });

  it('renders a list the Hub refused as an error, with no rows and nothing to add to, and reads it again when asked', async () => {
    let answer: (response: Response) => void = () => {};
    const { pending } = stubFetch([
      { method: 'GET', path: '/v1/passkeys', answer: json({ error: 'internal' }, 500) },
      {
        method: 'GET',
        path: '/v1/passkeys',
        answer: new Promise<Response>((resolve) => {
          answer = resolve;
        }),
      },
    ]);
    render(<Passkeys />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(screen.queryAllByRole('listitem')).toEqual([]);
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getAllByRole('button').map((button) => button.textContent)).toEqual(['Try again']);

    await click('Try again');

    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.getByText('Loading your passkeys…')).toBeInTheDocument();

    answer(json({ passkeys: [LAPTOP, PHONE] }, 200));

    expect(await statusOf('laptop')).toBe('Trusted');
    expect(await statusOf('phone')).toBe('Pending');
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Add passkey' })).toBeInTheDocument();
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('offers to read the list again each time the read fails', async () => {
    const { pending } = stubFetch([
      { method: 'GET', path: '/v1/passkeys', answer: json({ error: 'internal' }, 500) },
      { method: 'GET', path: '/v1/passkeys', answer: json({ error: 'unavailable' }, 503) },
    ]);
    render(<Passkeys />);
    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);

    await click('Try again');

    expect(await screen.findByRole('alert')).toHaveTextContent(/^unavailable$/);
    expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
    expect(pending).toEqual([]);
  });
});

describe('Passkeys, reading the list again after an action', () => {
  it('shows the list as the Hub read it after an endorsement, with a passkey endorsed elsewhere', async () => {
    const { sent, pending } = stubFetch([
      listing(LAPTOP, PHONE, DESK_KEY),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { passkey: { ...PHONE, status: 'trusted' } }),
      listing(LAPTOP, { ...PHONE, status: 'trusted' }, { ...DESK_KEY, status: 'trusted' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Endorse phone');

    await waitFor(async () => expect(await statusOf('desk key')).toBe('Trusted'));
    expect(await controlsOf('desk key')).toEqual([{ name: 'Revoke desk key', enabled: true }]);
    expect(await statusOf('phone')).toBe('Trusted');
    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: true }]);
    expect(sent.slice(3)).toEqual([{ path: '/v1/passkeys', body: undefined }]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('counts the only trusted passkey from the list read after the action, not from the one it held', async () => {
    const { pending } = stubFetch([
      listing(LAPTOP, TABLET, PHONE),
      post('/v1/passkeys/revoke/start', { ceremony_id: 'cer-3', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/revoke/finish', { passkey: { ...TABLET, status: 'revoked' } }),
      listing(LAPTOP, { ...TABLET, status: 'revoked' }, { ...PHONE, status: 'trusted' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Revoke tablet');

    await waitFor(async () => expect(await statusOf('phone')).toBe('Trusted'));
    expect(await statusOf('tablet')).toBe('Revoked');
    expect(await controlsOf('laptop')).toEqual([{ name: 'Revoke laptop', enabled: true }]);
    expect(pending).toEqual([]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('reads the list after a failed action too, and still shows what the action failed with', async () => {
    const { pending } = stubFetch([
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { error: 'internal' }, 500),
      listing(LAPTOP, { ...PHONE, status: 'trusted' }),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Endorse phone');

    await waitFor(async () => expect(await statusOf('phone')).toBe('Trusted'));
    expect(screen.getByRole('alert')).toHaveTextContent(/^internal$/);
    expect(pending).toEqual([]);
  });

  it('keeps the rows and shows no failure when the read after a successful action fails', async () => {
    const reread = unansweredListing();
    const { pending } = stubFetch([
      listing(LAPTOP, PHONE),
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { passkey: { ...PHONE, status: 'trusted' } }),
      reread.exchange,
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);
    await click('Endorse phone');
    await waitFor(async () => expect(await statusOf('phone')).toBe('Trusted'));
    expect(pending).toEqual([]);

    await reread.fail();

    expect(await statusOf('laptop')).toBe('Trusted');
    expect(await statusOf('phone')).toBe('Trusted');
    expect(screen.getAllByRole('listitem')).toHaveLength(2);
    expect(screen.getByRole('button', { name: 'Add passkey' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('keeps the rows and the failure of the action when the read after a failed action fails', async () => {
    const reread = unansweredListing();
    const { pending } = stubFetch([
      listing(LAPTOP, TABLET),
      post('/v1/passkeys/revoke/start', { error: 'last_passkey' }, 409),
      reread.exchange,
    ]);
    render(<Passkeys />);
    await click('Revoke tablet');
    expect(await screen.findByRole('alert')).toHaveTextContent(/^last_passkey$/);
    expect(pending).toEqual([]);

    await reread.fail();

    expect(screen.getAllByRole('alert')).toHaveLength(1);
    expect(screen.getByRole('alert')).toHaveTextContent(/^last_passkey$/);
    expect(await statusOf('laptop')).toBe('Trusted');
    expect(await statusOf('tablet')).toBe('Trusted');
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
  });

  it('does not put an older read over a newer one', async () => {
    let answerFirst: (response: Response) => void = () => {};
    const { pending } = stubFetch([
      listing(LAPTOP, TABLET, PHONE),
      post('/v1/passkeys/revoke/start', { error: 'internal' }, 500),
      {
        method: 'GET',
        path: '/v1/passkeys',
        answer: new Promise<Response>((resolve) => {
          answerFirst = resolve;
        }),
      },
      post('/v1/passkeys/endorse/start', { ceremony_id: 'cer-2', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/endorse/finish', { passkey: { ...PHONE, status: 'trusted' } }),
      listing(LAPTOP, TABLET, { ...PHONE, status: 'trusted' }, DESK_KEY),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);
    await click('Revoke tablet');
    await screen.findByRole('alert');

    await click('Endorse phone');
    await waitFor(async () => expect(await statusOf('desk key')).toBe('Pending'));
    expect(pending).toEqual([]);
    await act(async () => {
      answerFirst(json({ passkeys: [LAPTOP, TABLET, PHONE] }, 200));
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    expect(await statusOf('phone')).toBe('Trusted');
    expect(screen.getAllByRole('listitem')).toHaveLength(4);
  });
});

describe('Passkeys, when the Hub no longer holds the ceremony', () => {
  it('explains a ceremony_expired finish under the error, and the next attempt takes the explanation down', async () => {
    const { pending } = stubFetch([
      listing(LAPTOP, TABLET),
      post('/v1/passkeys/revoke/start', { ceremony_id: 'cer-3', options: REQUEST_OPTIONS }),
      post('/v1/passkeys/revoke/finish', { error: 'ceremony_expired' }, 400),
      listing(LAPTOP, TABLET),
      post('/v1/passkeys/revoke/start', { error: 'last_passkey' }, 409),
      listing(LAPTOP, TABLET),
    ]);
    webauthn.getAssertion.mockResolvedValueOnce(ASSERTION);
    render(<Passkeys />);

    await click('Revoke tablet');

    expect(await screen.findByRole('alert')).toHaveTextContent(/^ceremony_expired$/);
    const hint = screen.getByText(EXPIRED_CEREMONY_HINT);
    expect(hint).toHaveTextContent(/Try again\./);
    expect(hint).toHaveTextContent(
      /sign in with a different trusted passkey and revoke this one\.$/,
    );
    expect(await statusOf('tablet')).toBe('Trusted');

    await click('Revoke tablet');

    expect(await screen.findByText(/^last_passkey$/)).toHaveRole('alert');
    expect(screen.queryByText(EXPIRED_CEREMONY_HINT)).toBeNull();
    expect(pending).toEqual([]);
  });
});
