import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { listMachines, parseMachineAgents } from '../lib/machines';
import {
  checkLocalRunner,
  enableRemoteRuns,
  getRunnerStatus,
  type LocalRunnerCheck,
  type RunnerInstallStatus,
} from '../lib/runner';
import { invalidateRunnerCompatibility } from '../lib/runnerCompatibility';
import type { Machine, RunnerCompatibilityReport } from '../types';
import MachinesView from './MachinesView';

/** Every boundary call throws unless a test set up its answer, so a probe the
 *  view was not expected to make cannot pass by reading a default. */
function strict(name: string) {
  return vi.fn((...args: unknown[]): never => {
    throw new Error(`unexpected ${name}(${JSON.stringify(args)})`);
  });
}

vi.mock('../lib/machines', () => ({
  deleteMachine: strict('deleteMachine'),
  deleteMachineSecret: strict('deleteMachineSecret'),
  listMachines: strict('listMachines'),
  parseMachineAgents: strict('parseMachineAgents'),
  testMachineConnection: strict('testMachineConnection'),
}));

vi.mock('../lib/runner', () => ({
  cancelRunnerDownload: strict('cancelRunnerDownload'),
  checkLocalRunner: strict('checkLocalRunner'),
  downloadRunner: strict('downloadRunner'),
  enableRemoteRuns: strict('enableRemoteRuns'),
  getRunnerStatus: strict('getRunnerStatus'),
  getRunnerCompatibility: strict('getRunnerCompatibility'),
}));

vi.mock('../hooks/useTauriEvent', () => ({ useTauriEvent: () => {} }));

vi.mock('../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [], loading: false }),
  agentLabel: (_catalog: unknown, kind: string) => kind,
}));

vi.mock('../lib/runnerCompatibility', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../lib/runnerCompatibility')>()),
  invalidateRunnerCompatibility: vi.fn(),
}));

const machine: Machine = {
  id: 'machine-1',
  name: 'box',
  host: 'box.example',
  port: 22,
  username: 'dev',
  auth_type: 'agent',
};

const local: LocalRunnerCheck = {
  status: 'ready',
  path: '/cache/demeteo-runner',
  version: '1.2.0',
  expected: '1.2.0',
  stale_warning: null,
};

const behind: RunnerCompatibilityReport = {
  verdict: 'runner_behind',
  runner: '1.1.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable).',
};

const ahead: RunnerCompatibilityReport = {
  verdict: 'runner_ahead',
  runner: '1.3.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner 1.3.0 (stable) on box is newer than Demeteo 1.2.0 (stable).',
};

const compatible: RunnerCompatibilityReport = {
  verdict: 'compatible',
  version: '1.2.0',
  channel: 'stable',
  message: 'demeteo-runner 1.2.0 (stable) on box matches Demeteo.',
};

function installed(version: string, compatibility: RunnerCompatibilityReport): RunnerInstallStatus {
  return { installed: true, version, service_active: true, lingering: true, compatibility };
}

const pushButton = () =>
  screen.getByTitle('Download (if needed), push the latest build and restart the runner');

beforeEach(() => {
  vi.mocked(listMachines).mockResolvedValue([machine]);
  vi.mocked(parseMachineAgents).mockReturnValue([]);
  vi.mocked(checkLocalRunner).mockResolvedValue(local);
});

describe('MachinesView runner pill', () => {
  it('renders runner_behind from the verdict and offers an upgrade', async () => {
    vi.mocked(getRunnerStatus).mockResolvedValueOnce(installed('1.1.0', behind));
    render(<MachinesView />);

    expect(await screen.findByText(/update available/)).toBeTruthy();
    expect(screen.getByText('1.2.0 (stable)')).toBeTruthy();
    expect(pushButton().textContent).toBe('Upgrade runner');
  });

  it('labels the push a downgrade when the runner is ahead (D9)', async () => {
    vi.mocked(getRunnerStatus).mockResolvedValueOnce(installed('1.3.0', ahead));
    render(<MachinesView />);

    expect(await screen.findByText(/upgrade Demeteo/)).toBeTruthy();
    expect(pushButton().textContent).toBe('Downgrade runner to 1.2.0');
  });

  it('trusts a compatible verdict over a differing expected version string', async () => {
    const status = installed('demeteo-runner 1.2.0-31', compatible);
    vi.mocked(getRunnerStatus).mockResolvedValueOnce(status).mockResolvedValueOnce(status);
    vi.mocked(checkLocalRunner).mockResolvedValue({ ...local, expected: '1.2.0' });
    render(<MachinesView />);

    await screen.findByText('1.2.0 (stable)');
    fireEvent.click(screen.getByTitle('Check demeteo-runner status without installing'));
    await waitFor(() => expect(getRunnerStatus).toHaveBeenCalledTimes(2));
    await screen.findByText('1.2.0 (stable)');

    expect(screen.queryByText(/update available/)).toBeNull();
  });

  it('invalidates the cached verdict and re-probes after a push', async () => {
    vi.mocked(getRunnerStatus)
      .mockResolvedValueOnce(installed('1.1.0', behind))
      .mockResolvedValueOnce(installed('1.2.0', compatible));
    vi.mocked(enableRemoteRuns).mockResolvedValueOnce({
      version: '1.2.0',
      linger_enabled: true,
      warning: null,
    });
    render(<MachinesView />);

    await screen.findByText(/update available/);
    fireEvent.click(pushButton());

    await waitFor(() => expect(invalidateRunnerCompatibility).toHaveBeenCalledWith(machine.id));
    expect(enableRemoteRuns).toHaveBeenCalledWith(machine.id, local.path);
    await waitFor(() => expect(getRunnerStatus).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByText(/update available/)).toBeNull());
  });
});
