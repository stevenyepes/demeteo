import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { RunnerCompatibilityReport } from '../types';
import { LOCAL_MACHINE } from './newDiscovery';
import {
  blocksLaunch,
  cachedRunnerCompatibility,
  fetchRunnerCompatibility,
  invalidateRunnerCompatibility,
  isRunnerIncompatibleError,
  noticeFor,
  RUNNER_COMPATIBILITY_TTL_MS,
  shouldProbe,
} from './runnerCompatibility';

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

describe('isRunnerIncompatibleError', () => {
  it('accepts a runner_incompatible AppError carrying its verdict', () => {
    const err = { kind: 'runner_incompatible', message: behind.message, compatibility: behind };
    expect(isRunnerIncompatibleError(err)).toBe(true);
  });

  it('rejects other kinds, a missing verdict, plain strings and null', () => {
    expect(isRunnerIncompatibleError({ kind: 'validation', message: 'x', compatibility: behind })).toBe(
      false,
    );
    expect(isRunnerIncompatibleError({ kind: 'runner_incompatible', message: 'x' })).toBe(false);
    expect(
      isRunnerIncompatibleError({ kind: 'runner_incompatible', message: 'x', compatibility: {} }),
    ).toBe(false);
    expect(isRunnerIncompatibleError('runner_incompatible')).toBe(false);
    expect(isRunnerIncompatibleError(null)).toBe(false);
  });
});

describe('noticeFor', () => {
  it('says nothing for a compatible runner', () => {
    expect(noticeFor(compatible).tone).toBe('none');
  });

  it('blocks a runner behind the app and tells the user to upgrade the runner', () => {
    const notice = noticeFor(behind);
    expect(notice.tone).toBe('block');
    expect(notice.body.toLowerCase()).toContain('upgrade the runner');
  });

  it('blocks a runner ahead of the app and names both channels when they differ', () => {
    const notice = noticeFor(aheadAcrossChannels);
    expect(notice.tone).toBe('block');
    expect(notice.body.toLowerCase()).toContain('upgrade demeteo');
    expect(notice.body).toContain('nightly');
    expect(notice.body).toContain('stable');
  });

  it('blocks a machine with no runner installed', () => {
    expect(noticeFor(notInstalled).tone).toBe('block');
  });

  it('only informs when the version could not be verified', () => {
    const notice = noticeFor(unknown);
    expect(notice.tone).toBe('info');
    expect(notice.body).toContain('connection refused');
  });

  it('is silent about a missing runner on surfaces that never call it', () => {
    expect(noticeFor(notInstalled, 'informational').tone).toBe('none');
    expect(noticeFor(notInstalled, 'blocking').tone).toBe('block');
  });

  it('still reports a real mismatch, and an unverifiable runner, on informational surfaces', () => {
    expect(noticeFor(behind, 'informational').tone).toBe('block');
    expect(noticeFor(aheadAcrossChannels, 'informational').tone).toBe('block');
    expect(noticeFor(unknown, 'informational').tone).toBe('info');
    expect(noticeFor(compatible, 'informational').tone).toBe('none');
  });

  it('gives every non-silent verdict a distinct title', () => {
    const titles = [behind, aheadAcrossChannels, notInstalled, unknown].map((r) => noticeFor(r).title);
    expect(new Set(titles).size).toBe(titles.length);
    for (const title of titles) expect(title).not.toBe('');
  });
});

describe('blocksLaunch', () => {
  it('blocks only on a known mismatch or a missing runner', () => {
    expect(blocksLaunch(behind)).toBe(true);
    expect(blocksLaunch(aheadAcrossChannels)).toBe(true);
    expect(blocksLaunch(notInstalled)).toBe(true);
    expect(blocksLaunch(compatible)).toBe(false);
    expect(blocksLaunch(unknown)).toBe(false);
    expect(blocksLaunch(null)).toBe(false);
    expect(blocksLaunch(undefined)).toBe(false);
  });
});

describe('shouldProbe', () => {
  it('skips the desktop host and an unselected machine', () => {
    expect(shouldProbe('')).toBe(false);
    expect(shouldProbe(LOCAL_MACHINE)).toBe(false);
    expect(shouldProbe('m-1')).toBe(true);
  });
});

describe('runner compatibility cache', () => {
  const probes = () =>
    vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'remote_runner_compatibility');

  beforeEach(() => {
    vi.useFakeTimers();
    invalidateRunnerCompatibility();
    vi.mocked(invoke).mockImplementation(async (cmd) =>
      cmd === 'remote_runner_compatibility' ? behind : undefined,
    );
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('asks the backend for the machine it was given', async () => {
    await expect(fetchRunnerCompatibility('m-1')).resolves.toEqual(behind);
    expect(probes()).toEqual([['remote_runner_compatibility', { machineId: 'm-1' }]]);
    expect(cachedRunnerCompatibility('m-1')).toEqual(behind);
  });

  it('answers from the cache within the TTL and probes again after it', async () => {
    await fetchRunnerCompatibility('m-1');
    vi.advanceTimersByTime(RUNNER_COMPATIBILITY_TTL_MS - 1);
    await fetchRunnerCompatibility('m-1');
    expect(probes()).toHaveLength(1);

    vi.advanceTimersByTime(1);
    expect(cachedRunnerCompatibility('m-1')).toBeNull();
    await fetchRunnerCompatibility('m-1');
    expect(probes()).toHaveLength(2);
  });

  it('keys the cache by machine', async () => {
    await fetchRunnerCompatibility('m-1');
    await fetchRunnerCompatibility('m-2');
    expect(probes()).toHaveLength(2);
  });

  it('shares one probe between concurrent callers', async () => {
    await Promise.all([fetchRunnerCompatibility('m-1'), fetchRunnerCompatibility('m-1')]);
    expect(probes()).toHaveLength(1);
  });

  it('does not cache a failed probe', async () => {
    vi.mocked(invoke).mockRejectedValueOnce({ kind: 'transport', message: 'down' });
    await expect(fetchRunnerCompatibility('m-1')).rejects.toEqual({ kind: 'transport', message: 'down' });
    await fetchRunnerCompatibility('m-1');
    expect(probes()).toHaveLength(2);
  });

  it('invalidates one machine, or every machine with no argument', async () => {
    await fetchRunnerCompatibility('m-1');
    await fetchRunnerCompatibility('m-2');

    invalidateRunnerCompatibility('m-1');
    expect(cachedRunnerCompatibility('m-1')).toBeNull();
    expect(cachedRunnerCompatibility('m-2')).toEqual(behind);

    invalidateRunnerCompatibility();
    expect(cachedRunnerCompatibility('m-2')).toBeNull();
    await fetchRunnerCompatibility('m-1');
    await fetchRunnerCompatibility('m-2');
    expect(probes()).toHaveLength(4);
  });

  it('lets a caller bypass a fresh entry', async () => {
    await fetchRunnerCompatibility('m-1');
    await fetchRunnerCompatibility('m-1', { force: true });
    expect(probes()).toHaveLength(2);
  });
});
