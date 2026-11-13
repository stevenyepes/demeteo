import { invoke } from '@tauri-apps/api/core';
import { act, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HubLocalPolicy, HubStatus } from '../lib/hub';
import type { DeviceTicket, RunnerHubStatus, RunnerPairState } from '../lib/runnerHub';

const reportError = vi.fn();
vi.mock('../lib/errorBus', () => ({
  reportError: (...args: unknown[]) => reportError(...args),
}));

import { PAIR_POLL_MS, RunnerHubPanel } from './RunnerHubPanel';

const mockedInvoke = vi.mocked(invoke);
const MACHINE_ID = 'm-1';

afterEach(() => {
  reportError.mockReset();
  mockedInvoke.mockReset();
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
  keyring_backend: 'sealed',
  keyring_available: true,
};

const PAIRED: HubStatus = {
  ...UNPAIRED,
  state: 'paired',
  hub_url: 'https://hub.example.test',
  instance_id: 'inst-1',
  fingerprint: 'SHA256:Qm4x 7Hc2 8kPw',
};

function view(status: HubStatus): RunnerHubStatus {
  return {
    status,
    store_unavailable: null,
    connection: {
      state: status.state === 'paired' ? 'connected' : 'not_running',
      stop_reason: null,
      latency_ms: status.state === 'paired' ? 21 : null,
      reported_through: null,
      sync: status.state === 'paired' ? 'in_sync' : 'offline',
      last_heard_at_ms: null,
    },
    machine: { instance_name: 'build-box', os: 'linux', arch: 'x86_64', app_version: '1.4.0' },
  };
}

const TICKET: DeviceTicket = {
  user_code: 'WXYZ-1234',
  verification_uri: 'https://hub.example.test/device',
  verification_uri_complete: 'https://hub.example.test/device?user_code=WXYZ-1234',
  fingerprint: 'SHA256:runner key',
  expires_at: 1_790_000_600,
};

const POLICY: HubLocalPolicy = {
  read_enabled: true,
  spend_enabled: false,
  gates_enabled: false,
  run_ceiling_cents: 500,
  daily_ceiling_cents: null,
};

type Script = Record<string, (args?: unknown) => unknown>;

/** Every command must be scripted, and must name this panel's machine. */
function backend(script: Script) {
  const scripted: Script = {
    remote_hub_status: () => view(UNPAIRED),
    remote_hub_pair_state: () => ({ state: 'idle' }),
    remote_hub_policy_get: () => POLICY,
    remote_hub_request_log: () => [],
    ...script,
  };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    const answer = scripted[cmd];
    if (!answer) throw new Error(`unscripted invoke('${cmd}')`);
    if ((args as { machineId?: string } | undefined)?.machineId !== MACHINE_ID) {
      throw new Error(`invoke('${cmd}') without machineId ${MACHINE_ID}`);
    }
    return answer(args);
  }) as typeof invoke);
}

function callsTo(command: string) {
  return mockedInvoke.mock.calls.filter(([cmd]) => cmd === command);
}

async function openPanel() {
  render(<RunnerHubPanel machineId={MACHINE_ID} />);
  await userEvent.click(screen.getByRole('button', { name: /Hub pairing/ }));
}

describe('RunnerHubPanel', () => {
  it('reaches the runner only once opened', async () => {
    backend({});

    render(<RunnerHubPanel machineId={MACHINE_ID} />);

    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it('shows the ticket the runner answers: code, link and fingerprint', async () => {
    backend({ remote_hub_pair_begin: () => TICKET });

    await openPanel();
    await userEvent.type(await screen.findByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    expect(await screen.findByText('WXYZ-1234')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Open the Hub/ })).toHaveAttribute(
      'href',
      TICKET.verification_uri_complete,
    );
    expect(screen.getByText('SHA256:runner key')).toBeInTheDocument();
    expect(callsTo('remote_hub_pair_begin').map(([, a]) => a)).toEqual([
      { machineId: MACHINE_ID, hubUrl: 'https://hub.example.test' },
    ]);
  });

  it("shows the runner's own refusal to pair, unchanged", async () => {
    const refusal = 'cannot pair: no sealed key; run `demeteo-runner hub seal`';
    backend({
      remote_hub_pair_begin: () => {
        throw refusal;
      },
    });

    await openPanel();
    await userEvent.type(await screen.findByLabelText('Hub URL'), 'https://hub.example.test');
    await userEvent.click(screen.getByRole('button', { name: 'Pair' }));

    expect(await screen.findByText(refusal)).toBeInTheDocument();
  });

  it('polls the pairing every 2 s while pending and stops once it is idle', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    let pair: RunnerPairState = { state: 'pending', ticket: TICKET };
    let status = UNPAIRED;
    backend({
      remote_hub_pair_state: () => pair,
      remote_hub_status: () => view(status),
    });

    await openPanel();
    await screen.findByText('WXYZ-1234');
    const afterLoad = callsTo('remote_hub_pair_state').length;

    await act(async () => {
      vi.advanceTimersByTime(PAIR_POLL_MS);
    });
    expect(callsTo('remote_hub_pair_state').length).toBe(afterLoad + 1);

    pair = { state: 'idle' };
    status = PAIRED;
    await act(async () => {
      vi.advanceTimersByTime(PAIR_POLL_MS);
    });
    expect(await screen.findByText('https://hub.example.test')).toBeInTheDocument();
    const settled = callsTo('remote_hub_pair_state').length;

    await act(async () => {
      vi.advanceTimersByTime(PAIR_POLL_MS * 10);
    });
    expect(callsTo('remote_hub_pair_state').length).toBe(settled);
    expect(screen.queryByText('WXYZ-1234')).not.toBeInTheDocument();
  });

  it('toggling a scope sends the whole policy to the runner', async () => {
    backend({ remote_hub_policy_set: (args) => (args as { policy: HubLocalPolicy }).policy });

    await openPanel();
    const spend = await screen.findByRole('switch', { name: 'spend' });
    await waitFor(() => expect(spend).toBeEnabled());
    await userEvent.click(spend);

    await waitFor(() => expect(callsTo('remote_hub_policy_set')).toHaveLength(1));
    expect(callsTo('remote_hub_policy_set')[0][1]).toEqual({
      machineId: MACHINE_ID,
      policy: { ...POLICY, spend_enabled: true },
    });
    expect(spend).toHaveAttribute('aria-checked', 'true');
  });

  it('names an old runner instead of failing when it has no activity log', async () => {
    backend({ remote_hub_request_log: () => null });

    await openPanel();

    expect(
      await screen.findByText(/This runner is too old to report its Hub activity/),
    ).toBeInTheDocument();
    expect(reportError).not.toHaveBeenCalled();
  });

  it('names a runner from before Hub pairing', async () => {
    backend({ remote_hub_status: () => null });

    await openPanel();

    expect(await screen.findByText('This runner predates Hub pairing')).toBeInTheDocument();
  });

  it('says the Hub was not told when an older runner disconnects without a notice', async () => {
    let status = PAIRED;
    backend({
      remote_hub_status: () => view(status),
      remote_hub_disconnect: () => {
        status = UNPAIRED;
        return UNPAIRED;
      },
    });

    await openPanel();
    await userEvent.click(await screen.findByRole('button', { name: 'Disconnect from Hub' }));
    await userEvent.click(screen.getByRole('button', { name: 'Disconnect' }));

    const note = await screen.findByRole('status', { name: 'Hub not told' });
    expect(note).toHaveTextContent(/the Hub was not told/);
    expect(callsTo('remote_hub_disconnect')).toHaveLength(1);
  });
});
