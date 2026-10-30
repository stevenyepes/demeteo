import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { DevicePrompt } from '../lib/devicePairing';
import { ApproveDevice } from './ApproveDevice';

const device = vi.hoisted(() => ({
  lookUpDevice: vi.fn<(userCode: string) => Promise<unknown>>(),
  approveDevice: vi.fn<(userCode: string, scopes: string[]) => Promise<void>>(),
  denyDevice: vi.fn<(userCode: string) => Promise<void>>(),
}));

vi.mock('../lib/devicePairing', () => device);

const PROMPT: DevicePrompt = {
  userCode: 'BCDF-GHJK',
  fingerprint: 'SHA256:0ZcO CORZ NYy- DWpq',
  instanceName: 'build-01',
  os: 'linux',
  arch: 'x86_64',
  appVersion: '1.4.0',
  scopes: ['read', 'gates'],
  expiresAt: 1_700_000_600,
};

function refused(code: string, status = 400): HubApiError {
  return new HubApiError(`answered HTTP ${status}`, '/v1/pairing/device/lookup', status, code);
}

async function shown(typed = 'bcdfghjk'): Promise<void> {
  device.lookUpDevice.mockResolvedValueOnce(PROMPT);
  render(<ApproveDevice userCode={typed} />);
  await screen.findByRole('button', { name: 'Approve with passkey' });
}

beforeEach(() => {
  device.lookUpDevice.mockReset().mockRejectedValue(new Error('lookUpDevice was not scripted'));
  device.approveDevice.mockReset().mockRejectedValue(new Error('approveDevice was not scripted'));
  device.denyDevice.mockReset().mockRejectedValue(new Error('denyDevice was not scripted'));
});

afterEach(() => {
  window.location.hash = '';
});

describe('ApproveDevice', () => {
  it('without a code, asks for one and puts it in the hash rather than looking it up here', async () => {
    render(<ApproveDevice />);

    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
    await userEvent.type(screen.getByRole('textbox', { name: 'Code from the runner' }), '  bcdf-ghjk ');
    await userEvent.click(screen.getByRole('button', { name: 'Continue' }));

    expect(window.location.hash).toBe('#/device/bcdf-ghjk');
    expect(device.lookUpDevice).not.toHaveBeenCalled();
  });

  it('looks up the code it was given and shows the claims and the fingerprint as answered', async () => {
    await shown();

    expect(device.lookUpDevice.mock.calls).toEqual([['bcdfghjk']]);
    expect(screen.getByRole('region', { name: 'Device key fingerprint' })).toHaveTextContent(PROMPT.fingerprint);
    expect(screen.getByText('build-01')).toBeInTheDocument();
    expect(screen.getByText(/fingerprint the runner printed/)).toBeInTheDocument();
  });

  it("approves under the Hub's spelling of the code, with the scopes left checked", async () => {
    await shown();
    device.approveDevice.mockResolvedValueOnce();

    await userEvent.click(screen.getByRole('checkbox', { name: 'read' }));
    await userEvent.click(screen.getByRole('button', { name: 'Approve with passkey' }));

    expect(device.approveDevice.mock.calls).toEqual([['BCDF-GHJK', ['gates']]]);
    expect(await screen.findByRole('status')).toHaveTextContent(/Approved/);
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });

  it('offers no approval with every scope unchecked', async () => {
    await shown();

    await userEvent.click(screen.getByRole('checkbox', { name: 'read' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'gates' }));

    expect(screen.getByRole('button', { name: 'Approve with passkey' })).toBeDisabled();
  });

  it('denies without a passkey', async () => {
    await shown();
    device.denyDevice.mockResolvedValueOnce();

    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));

    expect(device.denyDevice.mock.calls).toEqual([['BCDF-GHJK']]);
    expect(device.approveDevice).not.toHaveBeenCalled();
    expect(await screen.findByRole('status')).toHaveTextContent(/Denied/);
  });

  it('leaves Approve to be tried again after a refused assertion', async () => {
    await shown();
    device.approveDevice.mockRejectedValueOnce(refused('assertion_invalid', 403));

    await userEvent.click(screen.getByRole('button', { name: 'Approve with passkey' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('assertion_invalid');
    expect(screen.getByRole('button', { name: 'Approve with passkey' })).toBeEnabled();
  });

  it.each([
    ['pairing_expired', 400, /No pending pairing has this code/],
    ['too_many_lookups', 429, /Wait ten minutes/],
    ['invalid_request', 400, /eight letters in two groups/],
  ])('after %s, says what to do and links back to the code entry', async (code, status, hint) => {
    device.lookUpDevice.mockRejectedValueOnce(refused(code, status));
    render(<ApproveDevice userCode="BCDF-GHJK" />);

    expect(await screen.findByRole('alert')).toHaveTextContent(code);
    expect(screen.getByText(hint)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Enter a different code' })).toHaveAttribute('href', '#/device');
    expect(screen.queryByRole('button', { name: 'Approve with passkey' })).toBeNull();
  });

  it('answers nothing more once an approval finds the code gone', async () => {
    await shown();
    device.approveDevice.mockRejectedValueOnce(refused('pairing_expired'));

    await userEvent.click(screen.getByRole('button', { name: 'Approve with passkey' }));

    expect(await screen.findByText(/No pending pairing has this code/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Approve with passkey' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled();
  });
});
