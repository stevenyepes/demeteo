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

/** What the two reads answer. The cached one is what a mount gets; the fetched
 *  one is what the press is paying for, and the point of the test is that the
 *  second one reaches the screen. */
const CACHED = drift(1, false);
const FETCHED = drift(9, true);

const driftCalls: boolean[] = [];

function mockBackend() {
  driftCalls.length = 0;
  vi.mocked(invoke).mockImplementation(((cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case 'feature_drift': {
        const refresh = args.refresh === true;
        driftCalls.push(refresh);
        return Promise.resolve(refresh ? FETCHED : CACHED);
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
  it('lands the count its own Refresh paid for', async () => {
    mount();

    await userEvent.click(await screen.findByRole('tab', { name: /Sync/ }));
    await waitFor(() => expect(behindMetric()).toEqual({ label: 'Behind (cached)', value: '1' }));

    const pane = screen.getByTestId('sync-panel');
    await userEvent.click(within(pane).getByRole('button', { name: 'Refresh' }));

    // The press pays for the fetch…
    await waitFor(() => expect(driftCalls).toContain(true));
    // …and the answer is what the pane ends up showing. Superseding the
    // in-flight read left the cached 1 on screen with no error anywhere.
    await waitFor(() => expect(behindMetric()).toEqual({ label: 'Behind', value: '9' }));
    expect(screen.getByRole('tab', { name: /Sync/ })).toHaveTextContent('Sync · 9');
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
