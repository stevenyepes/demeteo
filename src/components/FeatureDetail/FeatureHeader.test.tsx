// Telemetry-strip tests for FeatureHeader (UI_REDESIGN_PLAN §1 idea C).
//
// The value colours are asserted against `TONE_TEXT` rather than literal
// classes: a header that spells `text-emerald-400` itself passes a literal
// assertion while re-opening audit finding F27.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { runStatusMeta, TONE_TEXT } from '../../lib/runStatus';
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
      syncing={false}
      resolving={false}
      publishing={false}
      mrUrl={null}
      onBack={noop}
      onOpenTerminalTab={noop}
      onBrowseCode={noop}
      onCancelFeature={noop}
      onSync={noop}
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
