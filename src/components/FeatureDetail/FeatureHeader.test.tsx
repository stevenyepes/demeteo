// Telemetry-strip tests for FeatureHeader (UI_REDESIGN_PLAN §1 idea C).
//
// The value colours are asserted against `TONE_TEXT` rather than literal
// classes: a header that spells `text-emerald-400` itself passes a literal
// assertion while re-opening audit finding F27.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { runStatusMeta, TONE_TEXT } from '../../lib/runStatus';
import { REFRESH_HINT } from '../../lib/staleness';
import { FeatureHeader } from './FeatureHeader';

function renderHeader(overrides: Partial<Parameters<typeof FeatureHeader>[0]> = {}) {
  const noop = () => {};
  return render(
    <FeatureHeader
      featureId="feat-1"
      featureTitle="Add a metric strip"
      status="running"
      statusMeta={runStatusMeta('running')}
      currentProject={null}
      remoteRun={null}
      remoteMachineName={null}
      duration="10m 56s"
      totalCost={0.9834}
      tokens={600_700}
      cacheReadTokens={1_200_000}
      cacheCreationTokens={4_500}
      stepCount={7}
      drift={null}
      publishing={false}
      syncBadge={0}
      mrUrl={null}
      onBack={noop}
      onOpenTerminalTab={noop}
      onBrowseCode={noop}
      onCancelFeature={noop}
      onOpenSync={noop}
      onPublish={noop}
      onCleanup={noop}
      {...overrides}
    />,
  );
}

function metric(label: string): HTMLElement {
  const found = screen.getByTestId('metric-strip').querySelector(`[data-metric="${label}"]`);
  if (!(found instanceof HTMLElement)) throw new Error(`no metric labelled ${label}`);
  return found;
}

function metricValue(label: string): HTMLElement {
  const value = metric(label).querySelector('[data-testid="metric-value"]');
  if (!(value instanceof HTMLElement)) throw new Error(`metric ${label} has no value`);
  return value;
}

describe('FeatureHeader telemetry', () => {
  it('renders elapsed, cost, tokens and cache reads in one strip', () => {
    renderHeader();

    expect(screen.getByTestId('metric-strip')).toBeInTheDocument();
    expect(metricValue('Elapsed')).toHaveTextContent('10m 56s');
    expect(metricValue('Cost')).toHaveTextContent('$0.983');
    expect(metricValue('Tokens')).toHaveTextContent('600.7k');
    expect(metricValue('Cache Reads')).toHaveTextContent('1.2M');
  });

  it('drops cache reads entirely when the run served none', () => {
    renderHeader({ cacheReadTokens: 0 });

    expect(screen.getByTestId('metric-strip').querySelector('[data-metric="Cache Reads"]')).toBeNull();
    expect(screen.getAllByTestId('metric')).toHaveLength(3);
  });

  it('takes every value colour from the shared tone vocabulary', () => {
    renderHeader();

    expect(metricValue('Cost')).toHaveClass(TONE_TEXT.emerald);
    expect(metricValue('Tokens')).toHaveClass(TONE_TEXT.cyan);
    expect(metricValue('Cache Reads')).toHaveClass(TONE_TEXT.violet);
    expect(metricValue('Elapsed')).toHaveClass('text-white');
  });

  it('keeps the cost tooltip citing the unrounded total and the step count', () => {
    renderHeader();

    expect(metric('Cost')).toHaveAttribute('title', '0.9834 USD across 7 steps');
  });

  it('keeps the cache tooltip citing both the read and the written counters', () => {
    renderHeader();

    const title = metric('Cache Reads').getAttribute('title') ?? '';
    expect(title).toContain((1_200_000).toLocaleString());
    expect(title).toContain((4_500).toLocaleString());
  });
});

describe('FeatureHeader collapsed variant', () => {
  it('renders the full header when the caller says nothing about collapsing', () => {
    renderHeader();

    expect(screen.getByText('ID: feat-1')).toBeInTheDocument();
    expect(screen.getByTestId('feature-header')).toHaveClass('py-6');
    expect(screen.getByRole('heading', { name: 'Add a metric strip' })).toHaveClass('text-xl');
  });

  it('drops the id line and tightens the rhythm once collapsed', () => {
    renderHeader({ collapsed: true });

    expect(screen.queryByText('ID: feat-1')).toBeNull();
    expect(screen.getByTestId('feature-header')).toHaveClass('py-3');
    expect(screen.getByTestId('feature-header')).not.toHaveClass('py-6');
    expect(screen.getByRole('heading', { name: 'Add a metric strip' })).toHaveClass('text-lg');
  });

  it('keeps everything a scroll back to the top is for', () => {
    renderHeader({ collapsed: true });

    expect(screen.getByRole('heading', { name: 'Add a metric strip' })).toBeInTheDocument();
    expect(screen.getByText(runStatusMeta('running').label)).toBeInTheDocument();
    expect(screen.getByText('Local')).toBeInTheDocument();
    expect(screen.getByTestId('metric-strip')).toBeInTheDocument();
    expect(screen.getAllByTestId('metric')).toHaveLength(4);
    expect(screen.getByRole('button', { name: /code with agent/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /browse code/i })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /cancel feature/i })).toBeInTheDocument();
  });
});

/**
 * The header carries the two most prominent live pills in the app, and both
 * used to animate whole — label included — for the entire length of a run
 * (UI_REDESIGN_PLAN §6, Phase 5). `Chip` moves the pulse onto its dot; these
 * pin that it stayed there, in the one place where re-spelling the pill by hand
 * is most tempting.
 */
describe('FeatureHeader motion budget', () => {
  function chip(label: string): HTMLElement {
    const found = screen
      .getAllByTestId('chip')
      .find((c) => c.textContent?.includes(label));
    if (!found) throw new Error(`no chip labelled ${label}`);
    return found;
  }

  it('pulses the status dot while a run is live, never the pill around it', () => {
    renderHeader();

    const status = chip(runStatusMeta('running').label);
    expect(status.className).not.toMatch(/animate-pulse/);
    expect(status.querySelector('[data-testid="chip-dot"]')).toHaveClass('animate-pulse');
  });

  it('leaves a settled run with no animation at all', () => {
    renderHeader({ status: 'completed', statusMeta: runStatusMeta('completed') });

    const status = chip(runStatusMeta('completed').label);
    expect(status.className).not.toMatch(/animate-pulse/);
    expect(status.querySelector('[data-testid="chip-dot"]')).toBeNull();
  });

  it('pulses the transport dot only while a detached run is still going', () => {
    const mirror = (status: string) =>
      ({ machine_id: 'm-1', run_id: 'r-1', status }) as never;

    const live = renderHeader({ remoteRun: mirror('running'), remoteMachineName: 'gpu-box' });
    expect(chip('Remote · Detached').querySelector('[data-testid="chip-dot"]')).toHaveClass(
      'animate-pulse',
    );
    live.unmount();

    renderHeader({ remoteRun: mirror('completed'), remoteMachineName: 'gpu-box' });
    expect(chip('Remote · Detached').querySelector('[data-testid="chip-dot"]')).toBeNull();
  });
});

describe('FeatureHeader staleness', () => {
  function drift(behind: number | null) {
    return {
      divergence: { behind, ahead: 1 },
      base_ref: 'origin/main',
      fetched: true,
      checked_at: 0,
    };
  }

  function tones(): Record<string, string> {
    return Object.fromEntries(
      screen
        .getAllByTestId('chip')
        .map((c) => [c.textContent ?? '', c.getAttribute('data-tone') ?? '']),
    );
  }

  it('says nothing at all before a reading lands', () => {
    renderHeader({ status: 'completed', drift: null });
    expect(screen.queryByText(/behind/i)).toBeNull();
    expect(screen.queryByText(/up to date/i)).toBeNull();
  });

  it('names the commits a sync would pull in', () => {
    renderHeader({ status: 'completed', drift: drift(4) });
    expect(tones()['4 behind']).toBe('cyan');
  });

  it('keeps a branch nobody could measure out of the up-to-date state', () => {
    renderHeader({ status: 'completed', drift: drift(null) });
    expect(tones()['Drift unknown']).toBe('slate');
    expect(screen.queryByText(/up to date/i)).toBeNull();
  });

  it('calls a measured zero up to date', () => {
    renderHeader({ status: 'completed', drift: drift(0) });
    expect(tones()['Up to date']).toBe('emerald');
  });

  /**
   * The chip is the only affordance in the app that fetches `origin/<base>`
   * for a finished feature. Rendering it read-only leaves every count taken
   * against whatever ref an unrelated git flow last left behind — which is a
   * branch arbitrarily far behind trunk shown in emerald.
   */
  it('spends a press on a fetch of the base ref', async () => {
    const onRefreshDrift = vi.fn();
    renderHeader({ status: 'completed', drift: drift(0), onRefreshDrift });

    await userEvent.click(screen.getByTestId('drift-refresh'));

    expect(onRefreshDrift).toHaveBeenCalledTimes(1);
  });

  it('says the press is available in the tooltip, and only when it is', () => {
    const withPress = renderHeader({
      status: 'completed',
      drift: drift(0),
      onRefreshDrift: () => {},
    });
    expect(screen.getByTestId('drift-refresh')).toHaveAttribute(
      'title',
      expect.stringContaining(REFRESH_HINT) as unknown as string,
    );
    withPress.unmount();

    renderHeader({ status: 'completed', drift: drift(0) });
    expect(screen.getByTestId('drift-refresh').getAttribute('title')).not.toContain(REFRESH_HINT);
  });

  it('refuses a second press while the fetch it started is still in flight', async () => {
    const onRefreshDrift = vi.fn();
    renderHeader({
      status: 'completed',
      drift: drift(0),
      driftRefreshing: true,
      onRefreshDrift,
    });

    await userEvent.click(screen.getByTestId('drift-refresh'));

    expect(onRefreshDrift).not.toHaveBeenCalled();
  });
});

/**
 * The whole premise of one Sync pane is that it is reached through this button,
 * and its count is what advertises a conflict waiting on the header. Both were
 * asserted by nothing: replacing `onClick` with a no-op and flattening the badge
 * to a constant each left the suite green.
 */
describe('the header entry into the Sync pane', () => {
  it('opens the pane on a press', async () => {
    const onOpenSync = vi.fn();
    renderHeader({ status: 'completed', onOpenSync });

    await userEvent.click(screen.getByTestId('open-sync'));

    expect(onOpenSync).toHaveBeenCalledTimes(1);
  });

  it('carries the count the pane is waiting on, and none when it is zero', () => {
    const counted = renderHeader({ status: 'completed', syncBadge: 3 });
    expect(screen.getByTestId('open-sync')).toHaveTextContent('Sync · 3');
    counted.unmount();

    renderHeader({ status: 'completed', syncBadge: 0 });
    expect(screen.getByTestId('open-sync')).toHaveTextContent(/^Sync$/);
  });

  /** A run still writing to the branch owns its own sync, so there is no pane
   *  worth opening — and the button is absent rather than disabled. */
  it('offers no entry while the run is still going', () => {
    renderHeader({ status: 'running' });

    expect(screen.queryByTestId('open-sync')).toBeNull();
  });
});
