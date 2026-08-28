/**
 * The two joins between the Sync pane and the view that mounts it.
 *
 * `lib/syncPanel.ts` owns what each state says and `useSyncActions.test.tsx`
 * owns what each intent reaches for; neither can see the wiring here, and both
 * bugs this file pins lived in exactly that gap:
 *
 *   1. The pane's Refresh paid for a `git fetch` and then landed the cached
 *      count. `useFeatureDrift` supersedes its in-flight read whenever it is
 *      disabled, and the drift gate was disabled by *any* pending sync intent —
 *      including the refresh that had just started the fetch.
 *
 *   2. The header's Sync button only set the pane. Stacked, the inspector is
 *      rendered below the run surface and may be off-screen entirely, so the
 *      press produced nothing a user could see.
 *
 * The backend double answers only what this mount asks for and rejects the
 * rest, per `FeatureDetail.test.tsx`'s note on doubles that answer everything.
 */
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useEffect, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import {
  NavigationProvider,
  ProjectProvider,
  TerminalPanelProvider,
  UIStateProvider,
  useNavigation,
} from '../../context';
import type { FeatureDrift } from '../../types';
import { FeatureDetail } from './FeatureDetail';

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => <div>{children}</div>,
}));

const FEATURE_ID = 'f-1';

const drift = (behind: number, fetched: boolean): FeatureDrift => ({
  divergence: { behind, ahead: 0 },
  base_ref: 'origin/master',
  fetched,
  checked_at: 0,
});

/** What the two reads answer, in the order the pane makes them: the one it
 *  makes for itself on open, then the one a Refresh press pays for. Both
 *  fetch — the pane has no other way to be current — so the count is what tells
 *  them apart, and the point of the test is that the second one reaches the
 *  screen. */
const ON_OPEN = drift(9, true);
const ON_PRESS = drift(4, true);

const driftCalls: boolean[] = [];

function mockBackend() {
  driftCalls.length = 0;
  vi.mocked(invoke).mockImplementation(((cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case 'feature_drift': {
        const refresh = args.refresh === true;
        driftCalls.push(refresh);
        return Promise.resolve(driftCalls.length === 1 ? ON_OPEN : ON_PRESS);
      }
      case 'step_list_for_run':
        return Promise.resolve([]);
      case 'sync_session_get':
        return Promise.resolve(null);
      case 'feature_get':
        return Promise.resolve({ id: FEATURE_ID, status: 'completed' });
      case 'feature_workflow_graph':
      case 'get_app_session':
      case 'remote_run_for_feature':
        return Promise.resolve(null);
      case 'set_app_session':
        return Promise.resolve(undefined);
      case 'feature_list_attachments':
      case 'get_machines':
      case 'list_agents':
      case 'list_terminal_sessions':
      case 'step_attempts_list':
      case 'step_artifacts_list':
        return Promise.resolve([]);
      default:
        return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
    }
  }) as unknown as typeof invoke);

  vi.mocked(listen).mockImplementation((() =>
    Promise.resolve(() => {})) as unknown as typeof listen);
}

function Seed() {
  const { navigate } = useNavigation();
  useEffect(() => {
    navigate({ kind: 'detail', featureId: FEATURE_ID, featureTitle: 'Run' });
  }, [navigate]);
  return <FeatureDetail />;
}

function mount() {
  return render(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <TerminalPanelProvider>
            <Seed />
          </TerminalPanelProvider>
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );
}

/** The count as the pane states it, label included: `fetched` travels with the
 *  number precisely so a week-old one is not shown as this minute's. */
function behindMetric(): { label: string; value: string } {
  const strip = within(screen.getByTestId('sync-panel')).getByTestId('metric-strip');
  const metric = strip.querySelector('[data-metric^="Behind"]');
  if (!(metric instanceof HTMLElement)) throw new Error('the pane states no Behind count');
  return {
    label: metric.getAttribute('data-metric') ?? '',
    value: metric.querySelector('[data-testid="metric-value"]')?.textContent ?? '',
  };
}

beforeEach(() => {
  mockBackend();
});

describe('the Sync pane, wired into the run view', () => {
  it('fetches for the count it opens with, and lands the one Refresh paid for', async () => {
    mount();

    await userEvent.click(await screen.findByRole('tab', { name: /Sync/ }));
    // The pane's own read pays for the fetch, unasked. A count off an
    // unfetched ref is the previous fetch's answer, and the pane states it as
    // this one's — which is how it came to say a branch was level with a base
    // that had moved past it.
    await waitFor(() => expect(behindMetric()).toEqual({ label: 'Behind', value: '9' }));
    expect(driftCalls).not.toContain(false);

    const pane = screen.getByTestId('sync-panel');
    await userEvent.click(within(pane).getByRole('button', { name: 'Refresh' }));

    // The later answer is what the pane ends up showing. Superseding the
    // in-flight read left the opening 9 on screen with no error anywhere.
    await waitFor(() => expect(behindMetric()).toEqual({ label: 'Behind', value: '4' }));
    expect(screen.getByRole('tab', { name: /Sync/ })).toHaveTextContent('Sync · 4');
  });

  it('shows the pane the header button opens, rather than only selecting it', async () => {
    mount();
    const column = await screen.findByTestId('inspector-column');
    expect(column).toHaveAttribute('data-pane', 'step');

    await userEvent.click(screen.getByTestId('open-sync'));

    expect(column).toHaveAttribute('data-pane', 'sync');
    // Stacked, the pane can be below the fold; moving focus onto the inspector
    // wrapper — the same element `Enter` aims at — is what makes the press
    // visible there and takes the keyboard with it.
    expect(document.activeElement).not.toBe(document.body);
    expect(document.activeElement?.contains(column)).toBe(true);
  });
});
