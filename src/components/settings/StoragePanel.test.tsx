// The Storage tab's one destructive path: a scan on open, then a real sweep
// only behind a confirmation that names how many entries it will delete.
//
// `invoke` is mocked globally in `src/test/setup.ts`; anything unscripted
// throws, so a stray command surfaces as a failure.

import { invoke } from '@tauri-apps/api/core';
import { listen, type EventCallback } from '@tauri-apps/api/event';
import { act, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { StoragePanel } from './StoragePanel';
import { ProjectProvider } from '../../context';
import { resetCacheSweepSession, type CacheSweepProgress } from '../../lib/cacheSweepSession';
import type { CacheSweepReport } from '../../lib/featureDetail';

const mockedInvoke = vi.mocked(invoke);

function report(dryRun: boolean): CacheSweepReport {
  return {
    dry_run: dryRun,
    projects: [
      {
        project_id: 'p-1',
        clone_dir: '/w/demeteo',
        error: null,
        worktree_list_error: null,
        entries: [
          {
            path: '/w/demeteo-cache-feat-a',
            kind: 'cache',
            verdict: 'delete',
            reason: { code: 'feature_released', text: 'feature_released text' },
            outcome: dryRun ? null : { status: 'deleted' },
          },
          {
            path: '/w/demeteo-wt-b',
            kind: 'worktree',
            verdict: 'delete',
            reason: { code: 'unregistered', text: 'unregistered text' },
            outcome: dryRun ? null : { status: 'failed', detail: 'permission denied' },
          },
          {
            path: '/w/demeteo-cache-main',
            kind: 'cache',
            verdict: 'keep',
            reason: { code: 'sessions_recent', text: 'sessions_recent text' },
            outcome: null,
          },
        ],
      },
      {
        project_id: 'p-remote',
        clone_dir: null,
        error: 'ssh: connect to host failed',
        worktree_list_error: null,
        entries: [],
      },
    ],
  };
}

function sweepCalls(): boolean[] {
  return mockedInvoke.mock.calls
    .filter(([name]) => name === 'feature_cache_sweep')
    .map(([, args]) => (args as { dryRun: boolean }).dryRun);
}

beforeEach(() => {
  resetCacheSweepSession();
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    if (cmd === 'feature_cache_sweep') return report((args as { dryRun: boolean }).dryRun);
    throw new Error(`unscripted invoke('${cmd}')`);
  }) as typeof invoke);
});

describe('StoragePanel', () => {
  it('scans without deleting on open, and reports a project it could not reach', async () => {
    render(<ProjectProvider><StoragePanel /></ProjectProvider>);

    expect(await screen.findByText('/w/demeteo-cache-feat-a')).toBeInTheDocument();
    expect(sweepCalls()).toEqual([true]);
    expect(screen.getByText(/ssh: connect to host failed/)).toBeInTheDocument();
    expect(screen.getByText(/2 reclaimable/)).toBeInTheDocument();
    expect(screen.getByText('sessions_recent text')).toBeInTheDocument();
  });

  it('deletes only after a confirmation naming the count, then shows each outcome', async () => {
    render(<ProjectProvider><StoragePanel /></ProjectProvider>);
    await screen.findByText('/w/demeteo-cache-feat-a');

    await userEvent.click(screen.getByRole('button', { name: /Reclaim/ }));
    const confirm = screen.getByRole('alertdialog', { name: 'Confirm reclaim' });
    expect(confirm).toHaveTextContent('Delete the 2 entries');
    expect(sweepCalls()).toEqual([true]);

    await userEvent.click(within(confirm).getByRole('button', { name: 'Delete 2' }));

    expect(await screen.findByText(/Failed: permission denied/)).toBeInTheDocument();
    expect(screen.getByText('Deleted')).toBeInTheDocument();
    expect(sweepCalls()).toEqual([true, false]);
  });

  it('cancelling the confirmation deletes nothing', async () => {
    render(<ProjectProvider><StoragePanel /></ProjectProvider>);
    await screen.findByText('/w/demeteo-cache-feat-a');

    await userEvent.click(screen.getByRole('button', { name: /Reclaim/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Cancel' }));

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
    expect(sweepCalls()).toEqual([true]);
  });

  it('keeps a scan that finished after the tab was left, instead of starting another', async () => {
    let finish: (r: CacheSweepReport) => void = () => {};
    mockedInvoke.mockImplementation((async (cmd: string) => {
      if (cmd === 'feature_cache_sweep') return new Promise<CacheSweepReport>(resolve => { finish = resolve; });
      throw new Error(`unscripted invoke('${cmd}')`);
    }) as typeof invoke);

    const first = render(<ProjectProvider><StoragePanel /></ProjectProvider>);
    expect(screen.getByRole('status')).toHaveTextContent('Scanning every project');
    first.unmount();

    await act(async () => finish(report(true)));
    render(<ProjectProvider><StoragePanel /></ProjectProvider>);

    expect(await screen.findByText('/w/demeteo-cache-feat-a')).toBeInTheDocument();
    expect(sweepCalls()).toEqual([true]);
  });

  it('names the project a background sweep is on while a scan waits behind it', async () => {
    let emit: EventCallback<CacheSweepProgress> = () => {};
    vi.mocked(listen).mockImplementationOnce((async (_event: string, handler: EventCallback<CacheSweepProgress>) => {
      emit = handler;
      return () => {};
    }) as unknown as typeof listen);
    mockedInvoke.mockImplementation((async () => new Promise(() => {})) as typeof invoke);

    render(<ProjectProvider><StoragePanel /></ProjectProvider>);
    await act(async () => emit({
      event: 'cache_sweep_progress',
      id: 1,
      payload: { dry_run: false, project_id: 'p-9', project_index: 1, project_count: 3, deleting: '/w/x-cache-feat-z' },
    }));

    const status = screen.getByRole('status');
    expect(status).toHaveTextContent('Waiting for the background sweep, which is reclaiming project 2 of 3: p-9');
    expect(status).toHaveTextContent('Deleting /w/x-cache-feat-z');
  });
});
