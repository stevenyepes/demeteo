import { invoke } from '@tauri-apps/api/core';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  HubConnection,
  HubLocalPolicy,
  HubMachine,
  HubRequestLogEntry,
  HubStatus,
  HubTrustedPasskey,
} from '../../lib/hub';

const reportError = vi.fn();
vi.mock('../../lib/errorBus', () => ({
  reportError: (...args: unknown[]) => reportError(...args),
}));

import { HubTab } from './HubTab';

const mockedInvoke = vi.mocked(invoke);

afterEach(() => {
  reportError.mockReset();
  vi.useRealTimers();
});

const UNPAIRED: HubStatus = {
  state: 'unpaired',
  hub_url: null,
  instance_id: null,
  fingerprint: null,
  rp_id: null,
  scopes: [],
  paired_at: null,
  keyring_backend: 'Secret Service',
  keyring_available: true,
};

const PAIRED: HubStatus = {
  state: 'paired',
  hub_url: 'https://hub.example.test',
  instance_id: 'inst-1',
  fingerprint: 'SHA256:Qm4x 7Hc2 8kPw',
  rp_id: 'hub.example.test',
  scopes: ['read', 'spend', 'gates'],
  paired_at: 1_790_000_000,
  keyring_backend: 'Secret Service',
  keyring_available: true,
};

const NOT_RUNNING: HubConnection = {
  state: 'not_running',
  stop_reason: null,
  latency_ms: null,
  reported_through: null,
  sync: 'offline',
  last_heard_at_ms: null,
};

const CONNECTED: HubConnection = {
  state: 'connected',
  stop_reason: null,
  latency_ms: 38,
  reported_through: 9512,
  sync: 'in_sync',
  last_heard_at_ms: 1_790_000_000_000,
};

const DISPLACED: HubConnection = {
  state: 'stopped',
  stop_reason: 'displaced',
  latency_ms: null,
  reported_through: 9512,
  sync: 'offline',
  last_heard_at_ms: 1_790_000_000_000,
};

const MACHINE: HubMachine = {
  instance_name: 'studio-box',
  os: 'linux',
  arch: 'x86_64',
  app_version: '1.4.0',
};

const POLICY: HubLocalPolicy = {
  read_enabled: true,
  spend_enabled: true,
  gates_enabled: false,
  run_ceiling_cents: 500,
  daily_ceiling_cents: 2000,
};

const PINNED: HubTrustedPasskey = {
  credential_id: 'cred-pinned-0123456789abcdef',
  sign_count: 7,
  pinned_at: 1_790_000_000,
  origin: 'pinned',
};

const ENDORSED: HubTrustedPasskey = {
  credential_id: 'cred-endorsed-fedcba9876543210',
  sign_count: 2,
  pinned_at: 1_790_500_000,
  origin: 'endorsed',
};

const REFUSED_SPEND: HubRequestLogEntry = {
  id: 2,
  request_id: 'req-refused',
  kind: 'start_feature',
  scope: 'spend',
  outcome: 'refused',
  reason: 'scope_off',
  feature_id: null,
  received_at_ms: 1_790_000_100_000,
};

const ACCEPTED_GATE: HubRequestLogEntry = {
  id: 1,
  request_id: 'req-accepted',
  kind: 'gate_decision',
  scope: 'gates',
  outcome: 'accepted',
  reason: null,
  feature_id: 'f-1',
  received_at_ms: 1_790_000_000_000,
};

type Script = Record<string, (args?: unknown) => unknown>;

function backend(script: Script) {
  const scripted: Script = {
    hub_policy_get: () => POLICY,
    hub_policy_set: () => undefined,
    hub_trusted_passkeys: () => [],
    hub_request_log: () => [],
    ...script,
  };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const answer = scripted[cmd];
    if (!answer) throw new Error(`unscripted invoke('${cmd}')`);
    return answer(args);
  }) as typeof invoke);
}

function hub(status: HubStatus, connection: HubConnection, extra: Script = {}) {
  backend({
    hub_status: () => status,
    hub_connection: () => connection,
    hub_machine: () => MACHINE,
    ...extra,
  });
}

function policySets() {
  return mockedInvoke.mock.calls.filter(([cmd]) => cmd === 'hub_policy_set').map(([, a]) => a);
}

function callsTo(command: string) {
  return mockedInvoke.mock.calls.filter(([cmd]) => cmd === command).length;
}

function connectionCalls() {
  return callsTo('hub_connection');
}

describe('HubTab', () => {
  it('unpaired: offers a Hub URL and Pair, and shows no fingerprint', async () => {
    hub(UNPAIRED, NOT_RUNNING);

    render(<HubTab />);

    expect(await screen.findByLabelText('Hub URL')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Pair' })).toBeInTheDocument();
    expect(screen.queryByText(/SHA256:/)).not.toBeInTheDocument();
    expect(screen.queryByText(/never in SQLite/)).not.toBeInTheDocument();
  });

  it('paired-connected: emerald status with latency, fingerprint, keyring and cursor', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);

    expect(await screen.findByText('Connected · 38 ms')).toBeInTheDocument();
    expect(screen.getByTestId('hub-status-dot')).toHaveClass('bg-emerald-400', 'animate-pulse');
    expect(screen.getByText('https://hub.example.test')).toBeInTheDocument();
    expect(screen.getByText('SHA256:Qm4x 7Hc2 8kPw')).toBeInTheDocument();
    expect(screen.getByText('Kept in Secret Service · never in SQLite')).toBeInTheDocument();
    expect(screen.getByText(/Reported up to event #9,512/)).toHaveTextContent('in sync');
    expect(screen.getByText('This machine, as the Hub sees it')).toBeInTheDocument();
    expect(screen.getByText('studio-box')).toBeInTheDocument();
    expect(screen.getByText('linux · x86_64 · Demeteo 1.4.0')).toBeInTheDocument();
    expect(screen.getByText('hub.example.test')).toBeInTheDocument();
    expect(screen.queryByLabelText('Hub URL')).not.toBeInTheDocument();
  });

  it('paired-offline: ruby status, stop-reason copy, and latency shown as —', async () => {
    hub(PAIRED, DISPLACED);

    render(<HubTab />);

    expect(await screen.findByText('Stopped · —')).toBeInTheDocument();
    expect(screen.getByTestId('hub-status-dot')).toHaveClass('bg-ruby-400');
    expect(
      screen.getByText('Stopped: another connection for this instance took over'),
    ).toBeInTheDocument();
    expect(screen.getByText(/Reported up to event #9,512/)).toHaveTextContent('offline');
  });

  it('incomplete: says the pairing is partly missing and offers to pair again', async () => {
    hub({ ...PAIRED, state: 'incomplete' }, NOT_RUNNING, {
      hub_pair: () => PAIRED,
    });

    render(<HubTab />);

    expect(await screen.findByText(/pairing is incomplete/)).toHaveTextContent('Secret Service');
    await userEvent.click(screen.getByRole('button', { name: 'Pair again' }));
    expect(mockedInvoke).toHaveBeenCalledWith('hub_pair', { hubUrl: 'https://hub.example.test' });
  });

  it('says so when the keyring is unavailable', async () => {
    hub({ ...PAIRED, state: 'incomplete', keyring_available: false }, NOT_RUNNING);

    render(<HubTab />);

    expect(await screen.findByText(/Secret Service is unavailable/)).toBeInTheDocument();
    expect(screen.queryByText(/pairing is incomplete/)).not.toBeInTheDocument();
  });

  it('shows the not-available panel when hub_status is rejected', async () => {
    backend({
      hub_status: () => {
        throw new Error('Hub pairing is not available in this mode');
      },
    });

    render(<HubTab />);

    expect(await screen.findByText('Hub pairing is not available in this mode')).toBeInTheDocument();
    expect(screen.queryByLabelText('Hub URL')).not.toBeInTheDocument();
  });

  it('disables Pair while hub_pair is pending, then shows the pairing', async () => {
    let finishPairing: (status: HubStatus) => void = () => {};
    let connection = NOT_RUNNING;
    hub(UNPAIRED, NOT_RUNNING, {
      hub_connection: () => connection,
      hub_pair: () =>
        new Promise<HubStatus>((resolve) => {
          finishPairing = resolve;
        }),
    });

    render(<HubTab />);
    await userEvent.type(await screen.findByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    const waiting = await screen.findByRole('button', { name: /Waiting for the Hub/ });
    expect(waiting).toBeDisabled();
    expect(mockedInvoke).toHaveBeenCalledWith('hub_pair', { hubUrl: 'https://hub.example.test' });

    connection = CONNECTED;
    await act(async () => finishPairing(PAIRED));

    expect(await screen.findByText('Connected · 38 ms')).toBeInTheDocument();
    expect(reportError).not.toHaveBeenCalled();
  });

  it('reports a failed pairing and re-enables Pair', async () => {
    hub(UNPAIRED, NOT_RUNNING, {
      hub_pair: () => {
        throw new Error('the Hub refused the pairing');
      },
    });

    render(<HubTab />);
    await userEvent.type(await screen.findByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    await waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(screen.getByRole('button', { name: 'Pair' })).toBeEnabled();
  });

  it('polls hub_connection and hub_request_log every 5 s and stops on unmount', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    hub(PAIRED, CONNECTED);

    const { unmount } = render(<HubTab />);
    await screen.findByText('Connected · 38 ms');
    const afterLoad = connectionCalls();
    const logAfterLoad = callsTo('hub_request_log');

    await act(async () => {
      vi.advanceTimersByTime(5000);
    });
    expect(connectionCalls()).toBe(afterLoad + 1);
    expect(callsTo('hub_request_log')).toBe(logAfterLoad + 1);

    unmount();
    await act(async () => {
      vi.advanceTimersByTime(20_000);
    });
    expect(connectionCalls()).toBe(afterLoad + 1);
    expect(callsTo('hub_request_log')).toBe(logAfterLoad + 1);
  });

  it('lists a refused spend request with the human reason, unpaired', async () => {
    hub(UNPAIRED, NOT_RUNNING, { hub_request_log: () => [REFUSED_SPEND] });

    render(<HubTab />);

    const [row] = await screen.findAllByTestId('hub-activity-row');
    expect(within(row).getByText('spend is off on this desktop')).toBeInTheDocument();
    expect(within(row).getByText('refused')).toHaveClass('text-ruby-300');
    expect(within(row).getByText(/start feature/)).toBeInTheDocument();
    expect(within(row).queryByText('scope_off')).not.toBeInTheDocument();
  });

  it('shows a reason it has no copy for as recorded', async () => {
    hub(PAIRED, CONNECTED, {
      hub_request_log: () => [{ ...REFUSED_SPEND, reason: 'hub_on_fire' }],
    });

    render(<HubTab />);

    const [row] = await screen.findAllByTestId('hub-activity-row');
    expect(within(row).getByText('hub_on_fire')).toBeInTheDocument();
  });

  it('marks an accepted request with the emerald accepted pill', async () => {
    hub(PAIRED, CONNECTED, { hub_request_log: () => [ACCEPTED_GATE, REFUSED_SPEND] });

    render(<HubTab />);

    const rows = await screen.findAllByTestId('hub-activity-row');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('accepted')).toHaveClass(
      'bg-emerald-500/10',
      'text-emerald-300',
    );
    expect(within(rows[0]).getByText(/gate decision/)).toBeInTheDocument();
  });

  it('lists two rows that share a request_id without a duplicate-key warning', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const deferred: HubRequestLogEntry = {
      ...ACCEPTED_GATE,
      id: 1,
      outcome: 'deferred',
      received_at_ms: ACCEPTED_GATE.received_at_ms - 1_000,
    };
    hub(PAIRED, CONNECTED, { hub_request_log: () => [{ ...ACCEPTED_GATE, id: 2 }, deferred] });

    render(<HubTab />);

    const rows = await screen.findAllByTestId('hub-activity-row');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('accepted')).toBeInTheDocument();
    expect(within(rows[1]).getByText('deferred')).toBeInTheDocument();
    const keyWarnings = consoleError.mock.calls.filter((call) =>
      call.some((arg) => String(arg).includes('same key')),
    );
    consoleError.mockRestore();
    expect(keyWarnings).toEqual([]);
  });

  it('says no request has arrived when the log is empty', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);

    expect(await screen.findByText(/No request from the Hub has arrived yet/)).toBeInTheDocument();
    expect(screen.queryByTestId('hub-activity-row')).not.toBeInTheDocument();
  });

  it('renders exactly the read, spend and gates switches, and no logs switch', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);

    await screen.findByRole('switch', { name: /spend/i });
    const switches = screen.getAllByRole('switch');
    expect(switches).toHaveLength(3);
    expect(screen.queryByRole('switch', { name: /logs/i })).not.toBeInTheDocument();
    expect(screen.getByRole('switch', { name: /read/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('switch', { name: /spend/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('switch', { name: /gates/i })).toHaveAttribute('aria-checked', 'false');
  });

  it('says no Hub action can turn a toggle on, and what is never shared', async () => {
    hub(UNPAIRED, NOT_RUNNING);

    render(<HubTab />);

    expect(await screen.findByText(/No Hub action can turn a toggle on/)).toBeInTheDocument();
    expect(screen.getByText(/Never shared, whatever is on/)).toBeInTheDocument();
  });

  it('turning spend off saves the whole policy with only spend_enabled flipped', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);
    await userEvent.click(await screen.findByRole('switch', { name: /spend/i }));

    await waitFor(() => expect(policySets()).toEqual([{ policy: { ...POLICY, spend_enabled: false } }]));
    expect(screen.getByRole('switch', { name: /spend/i })).toHaveAttribute('aria-checked', 'false');
    expect(reportError).not.toHaveBeenCalled();
  });

  it('turning read off saves the policy with read_enabled flipped', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);
    await userEvent.click(await screen.findByRole('switch', { name: /read/i }));

    await waitFor(() => expect(policySets()).toEqual([{ policy: { ...POLICY, read_enabled: false } }]));
  });

  it('saving a ceiling sends the whole policy with only that ceiling changed', async () => {
    hub(PAIRED, CONNECTED);

    render(<HubTab />);
    const run = await screen.findByLabelText('Per-run maximum');
    await waitFor(() => expect(run).toHaveValue('5.00'));
    await userEvent.clear(run);
    await userEvent.type(run, '12.50{Enter}');

    await waitFor(() =>
      expect(policySets()).toEqual([{ policy: { ...POLICY, run_ceiling_cents: 1250 } }]),
    );
    expect(reportError).not.toHaveBeenCalled();
  });

  it('restores a rejected ceiling and reports the error', async () => {
    hub(PAIRED, CONNECTED, {
      hub_policy_set: () => {
        throw new Error('database is locked');
      },
    });

    render(<HubTab />);
    const run = await screen.findByLabelText('Per-run maximum');
    await waitFor(() => expect(run).toHaveValue('5.00'));
    await userEvent.clear(run);
    await userEvent.type(run, '12.50{Enter}');

    await waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(screen.getByLabelText('Per-run maximum')).toHaveValue('5.00');
  });

  it('lists a pinned and an endorsed passkey with their labels', async () => {
    hub(PAIRED, CONNECTED, { hub_trusted_passkeys: () => [PINNED, ENDORSED] });

    render(<HubTab />);

    const rows = await screen.findAllByTestId('hub-passkey');
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText('pinned')).toBeInTheDocument();
    expect(within(rows[0]).getByText('7 signatures')).toBeInTheDocument();
    expect(within(rows[0]).getByTitle(PINNED.credential_id)).toHaveClass('font-mono');
    expect(within(rows[1]).getByText('endorsed')).toBeInTheDocument();
    expect(within(rows[1]).getByText('2 signatures')).toBeInTheDocument();
  });

  it('lists the passkey a pairing pinned once Pair resolves', async () => {
    let passkeys: HubTrustedPasskey[] = [];
    hub(UNPAIRED, NOT_RUNNING, {
      hub_trusted_passkeys: () => passkeys,
      hub_pair: () => {
        passkeys = [PINNED];
        return PAIRED;
      },
    });

    render(<HubTab />);
    expect(await screen.findByText(/No Hub passkey is trusted yet/)).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    const row = await screen.findByTestId('hub-passkey');
    expect(within(row).getByTitle(PINNED.credential_id)).toBeInTheDocument();
    expect(within(row).getByText('pinned')).toBeInTheDocument();
    expect(screen.queryByText(/No Hub passkey is trusted yet/)).not.toBeInTheDocument();
    expect(reportError).not.toHaveBeenCalled();
  });

  it('pairing again replaces the listed passkeys with the newly pinned one', async () => {
    let passkeys = [ENDORSED];
    hub({ ...PAIRED, state: 'incomplete' }, NOT_RUNNING, {
      hub_trusted_passkeys: () => passkeys,
      hub_pair: () => {
        passkeys = [PINNED];
        return PAIRED;
      },
    });

    render(<HubTab />);
    await screen.findByTitle(ENDORSED.credential_id);
    await userEvent.click(screen.getByRole('button', { name: 'Pair again' }));

    expect(await screen.findByTitle(PINNED.credential_id)).toBeInTheDocument();
    expect(screen.getAllByTestId('hub-passkey')).toHaveLength(1);
    expect(screen.queryByTitle(ENDORSED.credential_id)).not.toBeInTheDocument();
  });

  it('reports a passkey reload that fails after pairing', async () => {
    let paired = false;
    hub(UNPAIRED, NOT_RUNNING, {
      hub_trusted_passkeys: () => {
        if (paired) throw new Error('pins unreadable');
        return [];
      },
      hub_pair: () => {
        paired = true;
        return PAIRED;
      },
    });

    render(<HubTab />);
    await userEvent.type(await screen.findByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    await waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(screen.getByText('SHA256:Qm4x 7Hc2 8kPw')).toBeInTheDocument();
  });

  it('says no passkey is trusted when the list is empty', async () => {
    hub(UNPAIRED, NOT_RUNNING);

    render(<HubTab />);

    expect(await screen.findByText(/No Hub passkey is trusted yet/)).toBeInTheDocument();
    expect(screen.queryByTestId('hub-passkey')).not.toBeInTheDocument();
  });

  it('carries no border-white class and keeps cyan off the card headings', async () => {
    const dropped: HubRequestLogEntry = { ...REFUSED_SPEND, id: 3, outcome: 'dropped' };
    const deferred: HubRequestLogEntry = { ...ACCEPTED_GATE, id: 4, outcome: 'deferred' };
    hub(PAIRED, CONNECTED, {
      hub_trusted_passkeys: () => [PINNED, ENDORSED],
      hub_request_log: () => [dropped, deferred],
    });

    const { container } = render(<HubTab />);
    await screen.findAllByTestId('hub-passkey');
    await screen.findByText('dropped');
    await screen.findByText('deferred');
    await userEvent.click(screen.getByRole('button', { name: 'Disconnect from Hub' }));
    screen.getByRole('alertdialog', { name: 'Confirm disconnect' });

    expect(container.querySelector('[class*="border-white"]')).toBeNull();
    expect(container.querySelector('h3 [class*="text-cyan"]')).toBeNull();
  });

  it('disconnect says runs keep going and cancelling it calls nothing', async () => {
    hub(PAIRED, CONNECTED, { hub_disconnect: () => UNPAIRED });

    render(<HubTab />);
    await userEvent.click(await screen.findByRole('button', { name: 'Disconnect from Hub' }));

    const confirm = screen.getByRole('alertdialog', { name: 'Confirm disconnect' });
    expect(within(confirm).getByText(/Runs keep going/)).toBeInTheDocument();
    expect(within(confirm).getByText(/forgets the device key and tokens/)).toBeInTheDocument();
    await userEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(callsTo('hub_disconnect')).toBe(0);
    expect(screen.getByText('SHA256:Qm4x 7Hc2 8kPw')).toBeInTheDocument();
  });

  it('confirming disconnect calls hub_disconnect and returns to the unpaired layout', async () => {
    let connection = CONNECTED;
    let passkeys = [PINNED];
    hub(PAIRED, CONNECTED, {
      hub_connection: () => connection,
      hub_trusted_passkeys: () => passkeys,
      hub_disconnect: () => {
        connection = NOT_RUNNING;
        passkeys = [];
        return UNPAIRED;
      },
    });

    render(<HubTab />);
    await screen.findAllByTestId('hub-passkey');
    await userEvent.click(await screen.findByRole('button', { name: 'Disconnect from Hub' }));
    const confirm = screen.getByRole('alertdialog', { name: 'Confirm disconnect' });
    await userEvent.click(within(confirm).getByRole('button', { name: 'Disconnect' }));

    expect(await screen.findByLabelText('Hub URL')).toBeInTheDocument();
    expect(callsTo('hub_disconnect')).toBe(1);
    expect(screen.queryByText(/SHA256:/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Disconnect from Hub' })).not.toBeInTheDocument();
    await waitFor(() => expect(screen.queryByTestId('hub-passkey')).not.toBeInTheDocument());
    expect(reportError).not.toHaveBeenCalled();
  });

  it('offers disconnect on an incomplete pairing but not when unpaired', async () => {
    hub({ ...PAIRED, state: 'incomplete' }, NOT_RUNNING);
    const { unmount } = render(<HubTab />);
    expect(await screen.findByRole('button', { name: 'Disconnect from Hub' })).toBeInTheDocument();
    unmount();

    hub(UNPAIRED, NOT_RUNNING);
    render(<HubTab />);
    await screen.findByLabelText('Hub URL');
    expect(screen.queryByRole('button', { name: 'Disconnect from Hub' })).not.toBeInTheDocument();
  });

  it('reports a rejected disconnect and keeps the pairing shown', async () => {
    hub(PAIRED, CONNECTED, {
      hub_disconnect: () => {
        throw new Error('keyring is locked');
      },
    });

    render(<HubTab />);
    await userEvent.click(await screen.findByRole('button', { name: 'Disconnect from Hub' }));
    await userEvent.click(screen.getByRole('button', { name: 'Disconnect' }));

    await waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    expect(screen.getByText('SHA256:Qm4x 7Hc2 8kPw')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Disconnect from Hub' })).toBeEnabled();
  });

  it('rolls a rejected toggle back and reports the error', async () => {
    let failSave: (e: Error) => void = () => {};
    hub(PAIRED, CONNECTED, {
      hub_policy_set: () =>
        new Promise<void>((_, reject) => {
          failSave = reject;
        }),
    });

    render(<HubTab />);
    const spend = await screen.findByRole('switch', { name: /spend/i });
    await userEvent.click(spend);
    expect(spend).toHaveAttribute('aria-checked', 'false');

    await act(async () => failSave(new Error('database is locked')));

    expect(screen.getByRole('switch', { name: /spend/i })).toHaveAttribute('aria-checked', 'true');
    expect(reportError).toHaveBeenCalledTimes(1);
  });

  it('disables every scope switch and ceiling while a save is pending', async () => {
    let finishSave: () => void = () => {};
    hub(PAIRED, CONNECTED, {
      hub_policy_set: () =>
        new Promise<void>((resolve) => {
          finishSave = resolve;
        }),
    });

    render(<HubTab />);
    const spend = await screen.findByRole('switch', { name: /spend/i });
    await waitFor(() => expect(spend).toBeEnabled());
    await userEvent.click(spend);

    const controls = () => [
      ...['read', 'spend', 'gates'].map((name) => screen.getByRole('switch', { name })),
      screen.getByLabelText('Per-run maximum'),
      screen.getByLabelText('Rolling 24-hour total'),
    ];
    for (const control of controls()) expect(control).toBeDisabled();
    await userEvent.click(screen.getByRole('switch', { name: 'gates' }));

    await act(async () => finishSave());

    for (const control of controls()) expect(control).toBeEnabled();
    expect(callsTo('hub_policy_set')).toBe(1);
  });

  it('shows the re-read policy after a rejected save and reports the error', async () => {
    let reads = 0;
    hub(PAIRED, CONNECTED, {
      hub_policy_get: () => {
        reads += 1;
        return reads === 1 ? POLICY : { ...POLICY, gates_enabled: true, run_ceiling_cents: 900 };
      },
      hub_policy_set: () => {
        throw new Error('database is locked');
      },
    });

    render(<HubTab />);
    await userEvent.click(await screen.findByRole('switch', { name: /spend/i }));

    await waitFor(() => expect(reportError).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(screen.getByRole('switch', { name: 'gates' })).toHaveAttribute('aria-checked', 'true'),
    );
    expect(screen.getByRole('switch', { name: /spend/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByLabelText('Per-run maximum')).toHaveValue('9.00');
    expect(callsTo('hub_policy_get')).toBe(2);
  });
});
