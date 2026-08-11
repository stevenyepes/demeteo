// Two contracts: the numbers, and the fact that typing in the project view's
// composer does not re-derive them.
//
// The render count goes through a pass-through `Metric` stub rather than an
// assertion that `ProjectTelemetry` is memoized, so an unstable prop that
// defeats the memo fails here too — the same shape `PipelineCard.test.tsx`
// uses for the row.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState, type ComponentProps, type ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ProjectTelemetry, summarizeProjectTelemetry } from './ProjectTelemetry';
import type { Feature } from '../types';

let metricRenders = 0;

vi.mock('./ui/MetricStrip', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./ui/MetricStrip')>();
  const Metric = (props: ComponentProps<typeof actual.Metric>) => {
    metricRenders += 1;
    return <actual.Metric {...props} />;
  };
  return { ...actual, Metric };
});

function feature(overrides: Partial<Feature> = {}): Feature {
  return {
    id: 'f-1',
    project_id: 'proj-1',
    title: 'Retry budget',
    description: '',
    status: 'completed',
    total_cost: 0,
    tokens: 0,
    duration: '1m',
    created_at: 1,
    ...overrides,
  };
}

function metricValueEl(label: string): Element {
  const value = screen
    .getByTestId('metric-strip')
    .querySelector(`[data-metric="${label}"] [data-testid="metric-value"]`);
  if (!value) throw new Error(`no metric labelled ${label}`);
  return value;
}

function metricValue(label: string): string {
  return metricValueEl(label).textContent ?? '';
}

beforeEach(() => {
  metricRenders = 0;
});

describe('summarizeProjectTelemetry', () => {
  it('counts only the runs that are still changing on their own', () => {
    const summary = summarizeProjectTelemetry([
      feature({ id: 'a', status: 'running' }),
      feature({ id: 'b', status: 'verifying' }),
      feature({ id: 'c', status: 'awaiting_gate' }),
      feature({ id: 'd', status: 'completed' }),
      feature({ id: 'e', status: 'failed' }),
    ]);

    expect(summary.active).toBe(2);
    expect(summary.total).toBe(5);
  });

  it('sums cost and tokens across the project, tolerating a missing count', () => {
    const summary = summarizeProjectTelemetry([
      feature({ id: 'a', total_cost: 1.5, tokens: 12_000 }),
      feature({ id: 'b', total_cost: 0.25, tokens: null }),
      feature({ id: 'c', total_cost: 0.25, tokens: 8_000 }),
    ]);

    expect(summary.costUsd).toBeCloseTo(2);
    expect(summary.tokens).toBe(20_000);
  });

  it('reads an empty project as zeros rather than as nothing to show', () => {
    expect(summarizeProjectTelemetry([])).toEqual({
      active: 0,
      total: 0,
      tokens: 0,
      costUsd: 0,
    });
  });
});

describe('ProjectTelemetry', () => {
  it('renders fleet, cost and tokens in one strip', () => {
    render(
      <ProjectTelemetry
        features={[
          feature({ id: 'a', status: 'running', total_cost: 1.5, tokens: 12_000 }),
          feature({ id: 'b', status: 'completed', total_cost: 0.5, tokens: 8_000 }),
        ]}
      />,
    );

    expect(metricValue('Fleet Active')).toBe('1');
    expect(metricValue('Cost')).toBe('$2.00');
    expect(metricValue('Tokens')).toBe('20k');
  });

  // AGENTS.md §4: emerald is healthy/spend, cyan is the stream vocabulary.
  // Every class here comes from `TONE_TEXT`, so a metric that grew its own
  // colour spelling fails rather than drifting (ux-audit F27).
  it('tones spend emerald and tokens cyan, and greys a fleet at rest', () => {
    const { rerender } = render(
      <ProjectTelemetry features={[feature({ id: 'a', status: 'running' })]} />,
    );
    expect(metricValueEl('Fleet Active')).toHaveClass('text-emerald-400');
    expect(metricValueEl('Cost')).toHaveClass('text-emerald-400');
    expect(metricValueEl('Tokens')).toHaveClass('text-cyan-400');

    rerender(<ProjectTelemetry features={[feature({ id: 'a', status: 'completed' })]} />);
    expect(metricValueEl('Fleet Active')).toHaveClass('text-slate-500');
    expect(metricValueEl('Cost')).toHaveClass('text-emerald-400');
  });

  it('does not re-derive the totals while a sibling composer is typed into', async () => {
    const features = [feature({ id: 'a', status: 'running', total_cost: 1, tokens: 1_000 })];

    function Harness(): ReactElement {
      const [text, setText] = useState('');
      return (
        <>
          <input aria-label="Composer" value={text} onChange={(e) => setText(e.target.value)} />
          <ProjectTelemetry features={features} />
        </>
      );
    }

    render(<Harness />);
    const afterMount = metricRenders;
    expect(afterMount).toBeGreaterThan(0);

    await userEvent.type(screen.getByLabelText('Composer'), 'add a retry budget');

    expect(metricRenders).toBe(afterMount);
  });
});
