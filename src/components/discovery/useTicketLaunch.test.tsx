import { invoke } from '@tauri-apps/api/core';
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { NavigationProvider, useNavigation } from '../../context';
import { ErrorBusProvider, useErrorBus } from '../../lib/errorBus';
import {
  cachedRunnerCompatibility,
  fetchRunnerCompatibility,
  invalidateRunnerCompatibility,
} from '../../lib/runnerCompatibility';
import type { TicketView } from '../../types';
import { useTicketLaunch } from './useTicketLaunch';

const ticket: TicketView = {
  ticket: {
    id: 't-1',
    discovery_id: 'd-1',
    seq: 1,
    title: 'Ticket 1',
    description: '',
    acceptance: [],
    files: [],
    blocked_by: [],
    test_command: null,
    workflow_id: null,
    agent_kind: null,
    model: null,
    effort: null,
    machine_id: 'm-box',
    attachments: [],
    state: 'unstarted',
    drop_reason: null,
    force_start_reason: null,
    force_started_at: null,
    feature_id: null,
    created_at: 0,
    updated_at: 0,
  },
  standing: { id: 't-1', lane: 'ready', startable: true, blockers: [] },
  placement: { placement: { kind: 'detached', machine_id: 'm-box' }, inherited: false },
  feature: null,
};

const refusal = {
  kind: 'runner_incompatible',
  message: 'The runner on box is 1.0.0; this Demeteo is 1.1.0. Upgrade the runner.',
  compatibility: {
    verdict: 'runner_behind',
    runner: '1.0.0',
    runner_channel: 'stable',
    app: '1.1.0',
    app_channel: 'stable',
  },
};

/** Rejects anything it was not told to answer, so a launch that called the
 *  wrong command cannot pass against a default. */
function backend(answers: Record<string, () => Promise<unknown>>): void {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd in answers ? answers[cmd]() : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
}

/** The slice of the view's `runAction` the hook relies on: the action runs,
 *  and its failure reaches the handler the hook passed. */
async function runAction(action: () => Promise<unknown>, reportFailure: (cause: unknown) => void) {
  try {
    await action();
  } catch (cause) {
    reportFailure(cause);
  }
}

/** The bus is module state, so a toast a failed test left behind would reach
 *  the next one; cleared after every test rather than at the end of each. */
let clearToasts: () => void = () => {};

function render() {
  const setActionError = vi.fn();
  const { result } = renderHook(
    () => ({
      launch: useTicketLaunch({ runAction, setActionError }),
      bus: useErrorBus(),
      nav: useNavigation(),
    }),
    {
      wrapper: ({ children }) => (
        <ErrorBusProvider>
          <NavigationProvider>{children}</NavigationProvider>
        </ErrorBusProvider>
      ),
    },
  );
  clearToasts = () => act(() => result.current.bus.clear());
  return { result, setActionError };
}

afterEach(() => {
  clearToasts();
  invalidateRunnerCompatibility();
});

describe('useTicketLaunch', () => {
  it('routes a runner_incompatible refusal to a toast offering machine settings, dropping the stale verdict', async () => {
    backend({
      remote_runner_compatibility: () =>
        Promise.resolve({ verdict: 'compatible', version: '1.1.0', channel: 'stable', message: 'ok' }),
      ticket_force_start: () => Promise.reject(refusal),
    });
    await fetchRunnerCompatibility('m-box');
    expect(cachedRunnerCompatibility('m-box')).not.toBeNull();
    const { result, setActionError } = render();

    await act(async () => result.current.launch.forceStart(ticket, 'blocker is moot'));

    expect(invoke).toHaveBeenCalledWith('ticket_force_start', expect.objectContaining({ ticketId: 't-1' }));
    expect(setActionError).not.toHaveBeenCalled();
    expect(cachedRunnerCompatibility('m-box')).toBeNull();
    const toast = result.current.bus.toasts[0];
    expect(toast).toMatchObject({ kind: 'runner_incompatible', message: refusal.message });
    expect(toast.action?.label).toBe('Open machine settings');

    act(() => toast.action?.onClick());

    expect(result.current.nav.view.kind).toBe('settings');
  });

  it('drops the verdict of the runner a one-launch override went to, not the stored one', async () => {
    backend({
      remote_runner_compatibility: () =>
        Promise.resolve({ verdict: 'compatible', version: '1.1.0', channel: 'stable', message: 'ok' }),
      ticket_start: () => Promise.reject(refusal),
    });
    await fetchRunnerCompatibility('m-box');
    await fetchRunnerCompatibility('m-other');
    const { result } = render();

    await act(async () => result.current.launch.start(ticket, 'm-other'));

    expect(invoke).toHaveBeenCalledWith('ticket_start', { ticketId: 't-1', machineId: 'm-other' });
    expect(cachedRunnerCompatibility('m-other')).toBeNull();
    expect(cachedRunnerCompatibility('m-box')).not.toBeNull();
  });

  it('puts any other failure in the banner and posts no toast', async () => {
    backend({ ticket_start: () => Promise.reject('ticket t-1 is locked') });
    const { result, setActionError } = render();

    await act(async () => result.current.launch.start(ticket));

    expect(setActionError).toHaveBeenCalledWith('ticket t-1 is locked');
    expect(result.current.bus.toasts).toEqual([]);
  });

  it('reports a started run whose credentials were parked as a notice that offers Runs', async () => {
    backend({
      ticket_start: () =>
        Promise.resolve({ id: 'f-1', title: 'the ticket', credentials_parked: 'unscripted rpc inject_credentials' }),
    });
    const { result, setActionError } = render();

    await act(async () => result.current.launch.start(ticket));

    expect(setActionError).not.toHaveBeenCalled();
    const toast = result.current.bus.toasts[0];
    expect(toast).toMatchObject({ kind: 'provider' });
    expect(toast.message).toContain('unscripted rpc inject_credentials');

    act(() => toast.action?.onClick());

    expect(result.current.nav.view.kind).toBe('remote-inbox');
  });
});
