import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { PairingPrompt, PairingScope } from '../lib/pairing';
import { ApprovePairing } from './ApprovePairing';

const pairing = vi.hoisted(() => ({
  startPairing: vi.fn<(request: string) => Promise<unknown>>(),
  approvePairing: vi.fn<(pairingId: string, scopes: string[]) => Promise<string>>(),
  denyPairing: vi.fn<(pairingId: string) => Promise<string>>(),
  leaveFor: vi.fn<(redirect: string) => void>(),
}));

vi.mock('../lib/pairing', () => pairing);

const REQUEST = 'eyJyZXF1ZXN0IjoiMSJ9';
const APPROVED = 'http://127.0.0.1:49152/callback?code=code-1&state=state-1';
const DENIED = 'http://127.0.0.1:49152/callback?error=access_denied&state=state-1';

const EXPIRED_HINT = /expired or was already answered\. Start pairing again from Demeteo on the machine\./;
const SPENT_HINT = /can no longer be answered\. Start pairing again from Demeteo on the machine\./;

function promptFor(scopes: PairingScope[], changed: Partial<PairingPrompt> = {}): PairingPrompt {
  return {
    pairingId: 'pairing-1',
    fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
    instanceName: 'workstation',
    os: 'linux',
    arch: 'x86_64',
    appVersion: '1.4.2',
    scopes,
    expiresAt: 1_700_000_300,
    ...changed,
  };
}

function refused(path: string, code: string | null, status = 400): HubApiError {
  return new HubApiError(`${path} answered HTTP ${status}`, path, status, code);
}

/** A call that stays in flight until the test settles it. */
function unanswered<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve: (value: T) => void = () => {};
  const promise = new Promise<T>((resolved) => {
    resolve = resolved;
  });
  return { promise, resolve };
}

/** Renders the view over a prompt asking for `scopes` and waits for it to be on the page. */
async function shown(scopes: PairingScope[], changed: Partial<PairingPrompt> = {}): Promise<void> {
  pairing.startPairing.mockResolvedValueOnce(promptFor(scopes, changed));
  render(<ApprovePairing request={REQUEST} />);
  await screen.findByRole('button', { name: 'Approve' });
}

function scopeBox(scope: PairingScope): HTMLElement {
  return screen.getByRole('checkbox', { name: scope });
}

async function click(name: 'Approve' | 'Deny'): Promise<void> {
  await userEvent.click(screen.getByRole('button', { name }));
}

/** Unscripted, every call to the Hub rejects: a view that asked for the wrong thing fails on that. */
beforeEach(() => {
  pairing.startPairing.mockReset().mockRejectedValue(new Error('startPairing was not scripted'));
  pairing.approvePairing.mockReset().mockRejectedValue(new Error('approvePairing was not scripted'));
  pairing.denyPairing.mockReset().mockRejectedValue(new Error('denyPairing was not scripted'));
  pairing.leaveFor.mockReset();
});

describe('ApprovePairing', () => {
  it('asks the Hub for the prompt of the request it was given, and shows the fingerprint as answered', async () => {
    await shown(['read']);

    expect(pairing.startPairing.mock.calls).toEqual([[REQUEST]]);
    const key = screen.getByRole('region', { name: 'Device key fingerprint' });
    const fingerprint = within(key).getByText('SHA256:0ZcO CORZ NYy- DWpq');
    expect(fingerprint).toHaveClass('font-mono');
    expect(key).toHaveTextContent(/matches the fingerprint Demeteo shows on the machine/);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows the instance name, OS, arch and version as claimed by the machine', async () => {
    await shown(['read']);

    const claimed = screen.getByRole('region', { name: 'Claimed by the machine' });
    expect(
      within(claimed)
        .getAllByRole('definition')
        .map((value) => value.textContent),
    ).toEqual(['workstation', 'linux', 'x86_64', '1.4.2']);
  });

  it('renders a claimed name that looks like markup as text', async () => {
    await shown(['read'], { instanceName: '<b>trusted</b>' });

    const claimed = screen.getByRole('region', { name: 'Claimed by the machine' });
    expect(within(claimed).getByText('<b>trusted</b>')).toBeInTheDocument();
    expect(claimed.querySelector('b')).toBeNull();
  });

  it('offers one checked checkbox per requested scope, saying what it allows, and none for a scope not requested', async () => {
    await shown(['read', 'gates']);

    const scopes = screen.getByRole('group', { name: 'Scopes requested' });
    expect(
      within(scopes)
        .getAllByRole('checkbox')
        .map((box) => box.getAttribute('id')),
    ).toEqual([scopeBox('read').id, scopeBox('gates').id]);
    expect(scopeBox('read')).toBeChecked();
    expect(scopeBox('gates')).toBeChecked();
    expect(scopeBox('read')).toHaveAccessibleDescription(/sends its events and snapshots/);
    expect(scopeBox('gates')).toHaveAccessibleDescription(/relay a gate decision/);
    expect(screen.queryByRole('checkbox', { name: 'spend' })).toBeNull();
    expect(screen.getAllByRole('checkbox')).toHaveLength(2);
  });

  it('Approve posts the scopes left checked, then leaves for the returned redirect', async () => {
    await shown(['read', 'spend', 'gates']);
    pairing.approvePairing.mockResolvedValueOnce(APPROVED);

    await click('Approve');

    expect(pairing.approvePairing.mock.calls).toEqual([['pairing-1', ['read', 'spend', 'gates']]]);
    expect(pairing.leaveFor.mock.calls).toEqual([[APPROVED]]);
    expect(pairing.denyPairing).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('approving with one scope unchecked posts the narrowed set', async () => {
    await shown(['read', 'spend', 'gates']);
    pairing.approvePairing.mockResolvedValueOnce(APPROVED);

    await userEvent.click(scopeBox('spend'));
    await click('Approve');

    expect(pairing.approvePairing.mock.calls).toEqual([['pairing-1', ['read', 'gates']]]);
    expect(pairing.leaveFor.mock.calls).toEqual([[APPROVED]]);
  });

  it('a scope unchecked and checked again is posted', async () => {
    await shown(['read', 'spend']);
    pairing.approvePairing.mockResolvedValueOnce(APPROVED);

    await userEvent.click(scopeBox('read'));
    await userEvent.click(scopeBox('read'));
    await click('Approve');

    expect(pairing.approvePairing.mock.calls).toEqual([['pairing-1', ['read', 'spend']]]);
  });

  it('Approve is disabled with no scope checked, and Deny is not', async () => {
    await shown(['read', 'gates']);

    await userEvent.click(scopeBox('read'));
    expect(screen.getByRole('button', { name: 'Approve' })).toBeEnabled();
    await userEvent.click(scopeBox('gates'));

    expect(screen.getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeEnabled();
    await click('Approve');
    expect(pairing.approvePairing).not.toHaveBeenCalled();
  });

  it('Deny posts the denial, then leaves for its redirect', async () => {
    await shown(['read', 'spend']);
    pairing.denyPairing.mockResolvedValueOnce(DENIED);

    await click('Deny');

    expect(pairing.denyPairing.mock.calls).toEqual([['pairing-1']]);
    expect(pairing.leaveFor.mock.calls).toEqual([[DENIED]]);
    expect(pairing.approvePairing).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('offers no second answer while one is in flight, nor once one has been given', async () => {
    await shown(['read']);
    const approval = unanswered<string>();
    pairing.approvePairing.mockReturnValueOnce(approval.promise);

    await click('Approve');

    expect(screen.getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
    expect(pairing.leaveFor).not.toHaveBeenCalled();

    approval.resolve(APPROVED);

    expect(await screen.findByText(/^Returning to Demeteo/)).toBeInTheDocument();
    expect(pairing.leaveFor.mock.calls).toEqual([[APPROVED]]);
    expect(screen.getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it.each([
    { code: 'invalid_scope', status: 400, alert: 'invalid_scope' },
    { code: 'too_many_pairings', status: 503, alert: 'too_many_pairings' },
    { code: 'internal', status: 500, alert: 'internal' },
    { code: null, status: 500, alert: '/v1/pairing/approve answered HTTP 500' },
  ])('offers no second answer once the Hub has spent the prompt on $alert', async ({ code, status, alert }) => {
    await shown(['read', 'gates']);
    pairing.approvePairing.mockRejectedValueOnce(refused('/v1/pairing/approve', code, status));

    await click('Approve');

    expect((await screen.findByRole('alert')).textContent).toBe(alert);
    expect(pairing.leaveFor).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
    expect(scopeBox('read')).toBeDisabled();
    expect(scopeBox('gates')).toBeDisabled();
    expect(screen.getByText(SPENT_HINT)).toBeInTheDocument();
    expect(screen.queryByText(EXPIRED_HINT)).toBeNull();
  });

  it.each([
    {
      failure: 'no response',
      error: new HubApiError('Request to /v1/pairing/approve failed: offline', '/v1/pairing/approve', null),
      alert: 'Request to /v1/pairing/approve failed: offline',
    },
    { failure: 'a 401', error: refused('/v1/pairing/approve', 'unauthenticated', 401), alert: 'unauthenticated' },
    { failure: 'the CSRF 403', error: refused('/v1/pairing/approve', 'csrf_mismatch', 403), alert: 'csrf_mismatch' },
    { failure: 'an unreadable body', error: refused('/v1/pairing/approve', 'invalid_request'), alert: 'invalid_request' },
  ])('can be answered again after $failure, which never reached the prompt', async ({ error, alert }) => {
    await shown(['read']);
    pairing.approvePairing.mockRejectedValueOnce(error).mockResolvedValueOnce(APPROVED);

    await click('Approve');

    expect((await screen.findByRole('alert')).textContent).toBe(alert);
    expect(pairing.leaveFor).not.toHaveBeenCalled();
    expect(screen.queryByText(SPENT_HINT)).toBeNull();
    expect(screen.queryByText(EXPIRED_HINT)).toBeNull();
    expect(screen.getByRole('button', { name: 'Approve' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeEnabled();
    expect(scopeBox('read')).toBeEnabled();

    await click('Approve');

    expect(pairing.approvePairing.mock.calls).toEqual([
      ['pairing-1', ['read']],
      ['pairing-1', ['read']],
    ]);
    expect(pairing.leaveFor.mock.calls).toEqual([[APPROVED]]);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each([
    { answer: 'Approve' as const, call: pairing.approvePairing, path: '/v1/pairing/approve' },
    { answer: 'Deny' as const, call: pairing.denyPairing, path: '/v1/pairing/deny' },
  ])('shows a prompt the Hub no longer holds as expired when $answer is refused', async ({ answer, call, path }) => {
    await shown(['read']);
    call.mockRejectedValueOnce(refused(path, 'pairing_expired'));

    await click(answer);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^pairing_expired$/);
    expect(screen.getByText(EXPIRED_HINT)).toBeInTheDocument();
    expect(pairing.leaveFor).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Approve' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it('renders a refused request as its code and offers nothing to approve', async () => {
    pairing.startPairing.mockRejectedValueOnce(refused('/v1/pairing/start', 'invalid_request'));
    render(<ApprovePairing request={REQUEST} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^invalid_request$/);
    expect(screen.queryAllByRole('button')).toEqual([]);
    expect(screen.queryAllByRole('checkbox')).toEqual([]);
  });

  it('does not call a request the Hub was too full to park spent', async () => {
    pairing.startPairing.mockRejectedValueOnce(refused('/v1/pairing/start', 'too_many_pairings', 503));
    render(<ApprovePairing request={REQUEST} />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^too_many_pairings$/);
    expect(screen.queryByText(SPENT_HINT)).toBeNull();
  });

  it('does not show the answer to a request it is no longer showing', async () => {
    const first = unanswered<PairingPrompt>();
    pairing.startPairing
      .mockReturnValueOnce(first.promise)
      .mockResolvedValueOnce(promptFor(['read'], { instanceName: 'second machine' }));
    const { rerender } = render(<ApprovePairing request="first-request" />);

    rerender(<ApprovePairing request="second-request" />);
    await screen.findByText('second machine');
    first.resolve(promptFor(['read'], { instanceName: 'first machine' }));
    // Everything the view does with the late answer is microtasks behind it.
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(pairing.startPairing.mock.calls).toEqual([['first-request'], ['second-request']]);
    expect(screen.getByText('second machine')).toBeInTheDocument();
    expect(screen.queryByText('first machine')).toBeNull();
  });
});
