// The bar is a binding, so these tests are about the wiring, not the policy:
// `pipelineFilter.test.ts` already owns what each segment means. What can break
// here is the typed value reaching `onChange`, the counts coming from
// `segmentCounts` rather than a second tally, the accessible names, and the
// division of the two empty states.

import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';

import {
  DEFAULT_PIPELINE_FILTER,
  filterPipelines,
  type PipelineFilterOptions,
  type PipelineRow,
} from '../lib/pipelineFilter';
import { TONE_TEXT } from '../lib/runStatus';
import { PipelineFilterBar } from './PipelineFilterBar';

type Row = PipelineRow & { id: string };

function row(id: string, status: string, over: Partial<Row> = {}): Row {
  return { id, status, created_at: 1, title: id, ...over };
}

const FEATURES: Row[] = [
  row('gate', 'gated', { title: 'Add SSH keepalive' }),
  row('creds', 'needs-credentials', { title: 'Rotate the runner key' }),
  row('run', 'running', { title: 'Windows path fence' }),
  row('done', 'completed', { title: 'Runner mirror' }),
];

function opts(over: Partial<PipelineFilterOptions> = {}): PipelineFilterOptions {
  return { ...DEFAULT_PIPELINE_FILTER, ...over };
}

function renderBar(
  value: PipelineFilterOptions,
  onChange: (next: PipelineFilterOptions) => void,
  over: { features?: Row[]; resultCount?: number } = {},
) {
  const features = over.features ?? FEATURES;
  return render(
    <PipelineFilterBar
      value={value}
      onChange={onChange}
      features={features}
      resultCount={over.resultCount ?? filterPipelines(features, value).length}
    />,
  );
}

/** The controlled contract: typing only accumulates if the parent feeds `value` back. */
function ControlledBar({ onChange }: { onChange: (next: PipelineFilterOptions) => void }) {
  const [value, setValue] = useState(DEFAULT_PIPELINE_FILTER);

  return (
    <PipelineFilterBar
      value={value}
      onChange={(next) => {
        setValue(next);
        onChange(next);
      }}
      features={FEATURES}
      resultCount={filterPipelines(FEATURES, value).length}
    />
  );
}

describe('PipelineFilterBar segments', () => {
  it('offers every segment, counted from the unfiltered list', () => {
    renderBar(opts(), () => {});

    expect(screen.getByRole('radiogroup', { name: 'Filter pipelines' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'All, 4' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Needs you, 2' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Active, 1' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Done, 1' })).toBeInTheDocument();
  });

  it('reports the segment as its typed value, leaving query and sort alone', async () => {
    const onChange = vi.fn<(next: PipelineFilterOptions) => void>();
    renderBar(opts({ query: 'ssh', sort: 'oldest' }), onChange);

    await userEvent.click(screen.getByRole('radio', { name: /Needs you/ }));

    expect(onChange).toHaveBeenCalledExactlyOnceWith({
      segment: 'needs-you',
      query: 'ssh',
      sort: 'oldest',
    });
  });

  it('marks the current segment checked', () => {
    renderBar(opts({ segment: 'active' }), () => {});

    expect(screen.getByRole('radio', { name: /Active/ })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /All/ })).toHaveAttribute('aria-checked', 'false');
  });

  // Amber is "a human is blocked" everywhere else (runStatus.ts); the count
  // that carries the app's core promise must not read as decoration.
  it('gives the needs-you count the amber a gate wears elsewhere', () => {
    renderBar(opts(), () => {});

    const badge = screen.getByRole('radio', { name: 'Needs you, 2' }).querySelector('span');
    expect(badge?.className).toContain(TONE_TEXT.amber);
  });

  it('keeps the counts unfiltered as the query narrows the list', () => {
    renderBar(opts({ query: 'keepalive' }), () => {});

    expect(screen.getByRole('radio', { name: 'All, 4' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Needs you, 2' })).toBeInTheDocument();
  });
});

describe('PipelineFilterBar query', () => {
  it('names the query field and reports what is typed into it', async () => {
    const onChange = vi.fn<(next: PipelineFilterOptions) => void>();
    render(<ControlledBar onChange={onChange} />);

    await userEvent.type(screen.getByRole('searchbox', { name: 'Filter pipelines by text' }), 'ssh');

    expect(onChange).toHaveBeenLastCalledWith(opts({ query: 'ssh' }));
  });

  it('offers no clear affordance until there is something to clear', () => {
    const { rerender } = renderBar(opts(), () => {});
    expect(screen.queryByRole('button', { name: 'Clear the filter text' })).not.toBeInTheDocument();

    rerender(
      <PipelineFilterBar
        value={opts({ query: 'ssh' })}
        onChange={() => {}}
        features={FEATURES}
        resultCount={1}
      />,
    );
    expect(screen.getByRole('button', { name: 'Clear the filter text' })).toBeInTheDocument();
  });

  it('clears only the query, leaving the segment and sort chosen', async () => {
    const onChange = vi.fn<(next: PipelineFilterOptions) => void>();
    renderBar(opts({ segment: 'done', query: 'ssh', sort: 'newest' }), onChange);

    await userEvent.click(screen.getByRole('button', { name: 'Clear the filter text' }));

    expect(onChange).toHaveBeenCalledExactlyOnceWith({
      segment: 'done',
      query: '',
      sort: 'newest',
    });
  });
});

describe('PipelineFilterBar sort', () => {
  it('names the sort control and shows the current order', () => {
    renderBar(opts({ sort: 'oldest' }), () => {});

    expect(screen.getByRole('combobox', { name: 'Sort pipelines' })).toHaveValue('oldest');
  });

  it('reports the chosen sort as its typed value', async () => {
    const onChange = vi.fn<(next: PipelineFilterOptions) => void>();
    renderBar(opts({ segment: 'active' }), onChange);

    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Sort pipelines' }), 'newest');

    expect(onChange).toHaveBeenCalledExactlyOnceWith({
      segment: 'active',
      query: '',
      sort: 'newest',
    });
  });
});

describe('PipelineFilterBar empty results', () => {
  it('says nothing while rows are showing', () => {
    renderBar(opts(), () => {});

    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('owns the case its own controls caused', () => {
    renderBar(opts({ query: 'nothing matches this' }), () => {});

    expect(screen.getByRole('status')).toHaveTextContent(/no pipelines match/i);
  });

  // A project with no features is not a filter outcome, and a reset would not
  // help; ProjectHome's EmptyStateCard owns it.
  it('stays quiet when the project has no features at all', () => {
    renderBar(opts(), () => {}, { features: [], resultCount: 0 });

    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('resets the narrowing choices without reverting the sort', async () => {
    const onChange = vi.fn<(next: PipelineFilterOptions) => void>();
    renderBar(opts({ segment: 'done', query: 'ssh', sort: 'oldest' }), onChange, { resultCount: 0 });

    await userEvent.click(screen.getByRole('button', { name: 'Clear filters' }));

    expect(onChange).toHaveBeenCalledExactlyOnceWith({
      segment: 'all',
      query: '',
      sort: 'oldest',
    });
  });
});
