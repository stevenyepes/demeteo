import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { RunnerInstallStatus } from '../lib/runner';
import { pushActionLabel } from '../lib/runnerCompatibility';
import type { RunnerCompatibilityReport } from '../types';
import { RunnerStatusPill } from './RunnerStatusPill';

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

const aheadAcrossChannels: RunnerCompatibilityReport = {
  verdict: 'runner_ahead',
  runner: '1.2.0-31',
  runner_channel: 'nightly',
  app: '1.2.0',
  app_channel: 'stable',
  message:
    "demeteo-runner 1.2.0-31 (nightly) on box is newer than Demeteo 1.2.0 (stable) — upgrade Demeteo to match, or push this app's runner from Machines settings.",
};

const unknown: RunnerCompatibilityReport = {
  verdict: 'unknown',
  app: '1.2.0',
  app_channel: 'stable',
  detail: 'unparseable version',
  message: "Couldn't verify the demeteo-runner version on box (unparseable version).",
};

const notInstalled: RunnerCompatibilityReport = {
  verdict: 'not_installed',
  app: '1.2.0',
  app_channel: 'stable',
  message: 'demeteo-runner is not installed on box.',
};

function status(
  compatibility: RunnerCompatibilityReport | null,
  version: string | null = 'demeteo-runner 1.1.0',
): RunnerInstallStatus {
  return {
    installed: version !== null,
    version,
    service_active: true,
    lingering: true,
    compatibility,
  };
}

function renderPill(s: RunnerInstallStatus) {
  return render(<RunnerStatusPill status={s} pushLabel="Upgrade runner" />);
}

describe('RunnerStatusPill', () => {
  it('shows no update or downgrade text for a compatible runner', () => {
    const { container } = renderPill(status(compatible, 'demeteo-runner 1.2.0'));

    expect(container).toHaveTextContent('Running');
    expect(screen.getByText('1.2.0 (stable)')).toHaveClass('font-mono');
    expect(container).not.toHaveTextContent(/update available/i);
    expect(container).not.toHaveTextContent(/upgrade Demeteo/i);
    expect(container).not.toHaveTextContent(/downgrade/i);
  });

  it('offers an update, naming the app version, when the runner is behind', () => {
    const { container } = renderPill(status(behind));

    expect(container).toHaveTextContent('update available');
    expect(screen.getByText('1.1.0 (stable)')).toHaveClass('font-mono');
    expect(screen.getByText('1.2.0 (stable)')).toHaveClass('font-mono');
    expect(container).not.toHaveTextContent(/upgrade Demeteo/i);
  });

  it('tells the user to upgrade Demeteo when the runner is ahead, with both channels', () => {
    const { container } = renderPill(status(aheadAcrossChannels));

    expect(container).toHaveTextContent('upgrade Demeteo');
    expect(container).not.toHaveTextContent(/update available/i);
    expect(screen.getByText('1.2.0-31 (nightly)')).toHaveClass('font-mono');
    expect(screen.getByText('1.2.0 (stable)')).toHaveClass('font-mono');
  });

  it('labels the push action as a downgrade only when the runner is ahead', () => {
    expect(pushActionLabel(aheadAcrossChannels, 'Upgrade runner')).toBe(
      'Downgrade runner to 1.2.0',
    );
    for (const report of [compatible, behind, unknown, notInstalled, null]) {
      expect(pushActionLabel(report, 'Upgrade runner')).toBe('Upgrade runner');
    }
  });

  it('names the push action it was given in the stopped-service hint', () => {
    render(
      <RunnerStatusPill
        status={{ ...status(aheadAcrossChannels), service_active: false }}
        pushLabel="Downgrade runner to 1.2.0"
      />,
    );

    expect(screen.getByText('Downgrade runner to 1.2.0')).toBeInTheDocument();
  });

  it('keeps an unverifiable runner neutral', () => {
    const { container } = renderPill(status(unknown, 'demeteo-runner ???'));

    expect(container).toHaveTextContent('demeteo-runner ???');
    expect(container).toHaveTextContent('version unverified');
    expect(container).not.toHaveTextContent(/update available|upgrade Demeteo|downgrade/i);
  });

  it.each([
    ['a not_installed verdict', status(notInstalled, null)],
    ['no verdict at all', status(null, null)],
  ])('reports the runner as not installed for %s', (_, s) => {
    const { container } = renderPill(s);

    expect(container).toHaveTextContent('Remote runner not installed');
    expect(container).not.toHaveTextContent(/update available|upgrade Demeteo|downgrade/i);
  });
});
