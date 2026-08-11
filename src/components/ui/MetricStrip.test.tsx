// Unit tests for the MetricStrip / Metric pair (UI_REDESIGN_PLAN §5.1).
//
// Three of these pin decisions rather than markup: tone resolves through
// `lib/runStatus.ts` (never a spelled-out colour class), the reserved value
// width never shrinks while a live run ticks, and a conditionally omitted
// metric costs no space.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { TONE_TEXT } from '../../lib/runStatus';
import { Metric, MetricStrip } from './MetricStrip';

function metricValue(): HTMLElement {
  return screen.getByTestId('metric-value');
}

describe('Metric', () => {
  it('renders the label and the caller-formatted value', () => {
    render(<Metric label="Elapsed" value="10m 56s" />);

    const metric = screen.getByTestId('metric');
    expect(metric).toHaveTextContent('Elapsed');
    expect(metric).toHaveTextContent('10m 56s');
  });

  it('sets the value in the mono font with tabular figures', () => {
    render(<Metric label="Cost" value="$0.983" />);

    expect(metricValue()).toHaveClass('font-mono', 'tabular-nums', 'whitespace-nowrap');
  });

  it('takes the value colour from the shared tone vocabulary', () => {
    render(<Metric label="Tokens" value="600.7K" tone="violet" />);

    expect(metricValue()).toHaveClass(TONE_TEXT.violet);
  });

  it('leaves an untoned value neutral rather than picking a status colour', () => {
    render(<Metric label="Elapsed" value="10m 56s" />);

    const value = metricValue();
    expect(value).toHaveClass('text-white');
    for (const toneClass of Object.values(TONE_TEXT)) {
      expect(value).not.toHaveClass(toneClass);
    }
  });

  it('exposes the tooltip on hover', () => {
    render(<Metric label="Cache Reads" value="1.2M" tooltip="Served from prompt cache" />);

    expect(screen.getByTestId('metric')).toHaveAttribute('title', 'Served from prompt cache');
  });

  it('never narrows the value once a wider one has been shown', () => {
    const { rerender } = render(<Metric label="Cost" value="$0.98" />);
    expect(metricValue().style.minWidth).toBe('5ch');

    rerender(<Metric label="Cost" value="$10.98" />);
    expect(metricValue().style.minWidth).toBe('6ch');

    rerender(<Metric label="Cost" value="$9.98" />);
    expect(metricValue().style.minWidth).toBe('6ch');
  });

  it('drops the reserved width when the metric becomes a different one', () => {
    const { rerender } = render(<Metric label="Tokens" value="600.7K" />);
    expect(metricValue().style.minWidth).toBe('6ch');

    rerender(<Metric label="Cost" value="$0.98" />);
    expect(metricValue().style.minWidth).toBe('5ch');
  });
});

describe('MetricStrip', () => {
  it('groups the metrics it is given', () => {
    render(
      <MetricStrip>
        <Metric label="Elapsed" value="10m 56s" />
        <Metric label="Cost" value="$0.983" />
      </MetricStrip>,
    );

    expect(screen.getAllByTestId('metric')).toHaveLength(2);
  });

  it('renders nothing at all for an omitted metric', () => {
    const cacheReads = 0;
    render(
      <MetricStrip>
        <Metric label="Elapsed" value="10m 56s" />
        {cacheReads > 0 && <Metric label="Cache Reads" value="0" />}
        <Metric label="Cost" value="$0.983" />
      </MetricStrip>,
    );

    const strip = screen.getByTestId('metric-strip');
    expect(strip.childElementCount).toBe(2);
  });

  it('wraps instead of forcing the header wider than its column', () => {
    render(
      <MetricStrip>
        <Metric label="Elapsed" value="10m 56s" />
      </MetricStrip>,
    );

    expect(screen.getByTestId('metric-strip')).toHaveClass('flex-wrap', 'min-w-0');
  });

  it('merges a caller-supplied className', () => {
    render(
      <MetricStrip className="ml-2">
        <Metric label="Elapsed" value="10m 56s" />
      </MetricStrip>,
    );

    expect(screen.getByTestId('metric-strip')).toHaveClass('ml-2');
  });
});
