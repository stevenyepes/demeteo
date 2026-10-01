import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { RunnerCompatibilityReport } from '../types';
import { RunnerVersionNotice } from './RunnerVersionNotice';

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock('../context', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../context')>()),
  useNavigation: () => ({ navigate }),
}));

const compatible: RunnerCompatibilityReport = {
  verdict: 'compatible',
  version: '1.2.0',
  channel: 'stable',
  message: 'demeteo-runner 1.2.0 (stable) on box matches Demeteo.',
};

const behind: RunnerCompatibilityReport = {
  verdict: 'runner_behind',
  runner: '1.1.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message:
    'demeteo-runner 1.1.0 (stable) on box is older than Demeteo 1.2.0 (stable) — upgrade the runner from Machines settings.',
};

const ahead: RunnerCompatibilityReport = {
  verdict: 'runner_ahead',
  runner: '1.3.0',
  runner_channel: 'stable',
  app: '1.2.0',
  app_channel: 'stable',
  message:
    "demeteo-runner 1.3.0 (stable) on box is newer than Demeteo 1.2.0 (stable) — upgrade Demeteo to match, or push this app's runner from Machines settings.",
};

const aheadAcrossChannels: RunnerCompatibilityReport = {
  verdict: 'runner_ahead',
  runner: '1.2.0-31',
  runner_channel: 'nightly',
  app: '1.2.0',
  app_channel: 'stable',
  message:
    "demeteo-runner 1.2.0-31 (nightly) on box is newer than Demeteo 1.2.0 (stable) — upgrade Demeteo to match, or push this app's runner from Machines settings. The runner is on the nightly channel and Demeteo on stable; both must be on the same channel and version.",
};

const notInstalled: RunnerCompatibilityReport = {
  verdict: 'not_installed',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner is not installed on box — enable remote runs from Machines settings.',
};

const unknown: RunnerCompatibilityReport = {
  verdict: 'unknown',
  app: '1.2.0',
  app_channel: 'stable',
  detail: 'ssh: connection refused',
  message:
    "Couldn't verify the demeteo-runner version on box (ssh: connection refused) — check the machine in Machines settings.",
};

beforeEach(() => navigate.mockReset());

describe('RunnerVersionNotice', () => {
  it('tells the user to upgrade the runner when it is behind', () => {
    render(<RunnerVersionNotice report={behind} variant="blocking" />);

    const notice = screen.getByRole('alert');
    expect(notice).toHaveTextContent('Runner is older than Demeteo');
    expect(notice).toHaveTextContent('upgrade the runner');
    expect(notice.className).toContain('ruby');
  });

  it('tells the user to upgrade Demeteo when the runner is ahead', () => {
    render(<RunnerVersionNotice report={ahead} variant="blocking" />);

    expect(screen.getByRole('alert')).toHaveTextContent('upgrade Demeteo');
  });

  it('names both channels when they differ', () => {
    render(<RunnerVersionNotice report={aheadAcrossChannels} variant="blocking" />);

    const notice = screen.getByRole('alert');
    expect(notice).toHaveTextContent('nightly');
    expect(notice).toHaveTextContent('stable');
    expect(screen.getByText('1.2.0-31 (nightly)')).toHaveClass('font-mono');
  });

  it('renders an unverifiable runner as a neutral notice, not an error', () => {
    render(<RunnerVersionNotice report={unknown} variant="blocking" />);

    const notice = screen.getByRole('status');
    expect(notice).toHaveTextContent("Couldn't verify the runner version");
    expect(notice.className).toContain('amber');
    expect(notice.className).not.toContain('ruby');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each([
    ['a compatible runner', compatible],
    ['no verdict yet', null],
  ])('renders nothing for %s', (_, report) => {
    const { container } = render(<RunnerVersionNotice report={report} variant="blocking" />);

    expect(container).toBeEmptyDOMElement();
  });

  it('blocks on a missing runner in the blocking variant', () => {
    render(<RunnerVersionNotice report={notInstalled} variant="blocking" />);

    expect(screen.getByRole('alert')).toHaveTextContent('Runner not installed');
  });

  it('says nothing about a missing runner in the informational variant', () => {
    const { container } = render(<RunnerVersionNotice report={notInstalled} variant="informational" />);

    expect(container).toBeEmptyDOMElement();
  });

  it('opens machine settings from the notice', () => {
    render(<RunnerVersionNotice report={behind} variant="blocking" />);

    fireEvent.click(screen.getByRole('button', { name: /open machine settings/i }));
    expect(navigate).toHaveBeenCalledWith({ kind: 'settings' });
  });

  it('hands the settings link to its host instead of navigating when given one', () => {
    const onOpenSettings = vi.fn();
    render(<RunnerVersionNotice report={behind} variant="blocking" onOpenSettings={onOpenSettings} />);

    fireEvent.click(screen.getByRole('button', { name: /open machine settings/i }));
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('informs without alarming in the informational variant, and still links to settings', () => {
    render(<RunnerVersionNotice report={behind} variant="informational" />);

    const notice = screen.getByRole('status');
    expect(notice).toHaveTextContent('upgrade the runner');
    expect(notice.className).toContain('amber');
    expect(notice.className).not.toContain('ruby');

    fireEvent.click(screen.getByRole('button', { name: /open machine settings/i }));
    expect(navigate).toHaveBeenCalledWith({ kind: 'settings' });
  });

  it('explains that launch is disabled while the notice blocks', () => {
    render(<RunnerVersionNotice report={behind} variant="blocking" />);

    expect(screen.getByRole('alert')).toHaveTextContent(/launch stays disabled until the runner/i);
  });

  it('re-checks the runner on request', () => {
    const onRecheck = vi.fn();
    render(<RunnerVersionNotice report={behind} variant="blocking" onRecheck={onRecheck} />);

    fireEvent.click(screen.getByRole('button', { name: /check again/i }));
    expect(onRecheck).toHaveBeenCalledOnce();
  });

  it('disables re-checking while a check is in flight', () => {
    const onRecheck = vi.fn();
    render(<RunnerVersionNotice report={behind} variant="blocking" onRecheck={onRecheck} checking />);

    const button = screen.getByRole('button', { name: /check again/i });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(onRecheck).not.toHaveBeenCalled();
  });

  it.each([
    ['an informational notice', behind, 'informational'],
    ['an unverifiable runner', unknown, 'blocking'],
  ] as const)('offers no re-check or launch explanation on %s', (_, report, variant) => {
    render(<RunnerVersionNotice report={report} variant={variant} onRecheck={vi.fn()} />);

    expect(screen.queryByRole('button', { name: /check again/i })).toBeNull();
    expect(screen.getByRole('status')).not.toHaveTextContent(/launch stays disabled/i);
  });
});
