// Unit tests for `TaskListArtifact` — the presentational card renderer for a
// `task-list.json` artifact.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { TaskPlan } from '../../types';
import { TaskListArtifact } from './TaskListArtifact';

describe('TaskListArtifact', () => {
  it('renders one card per task with no cycle heading for a greenfield single-cycle plan', () => {
    const plan: TaskPlan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [
        {
          id: 't1',
          title: 'First task',
          description: 'Do the first thing',
          files: ['src/foo.ts'],
          acceptance: ['Foo works'],
          blocked_by: [],
          test_command: 'npm test -- foo',
        },
        { id: 't2', title: 'Second task', description: 'Do the second thing', blocked_by: ['t1'] },
      ],
    };

    render(<TaskListArtifact plan={plan} />);

    expect(screen.getByText('First task')).toBeInTheDocument();
    expect(screen.getByText('Do the first thing')).toBeInTheDocument();
    expect(screen.getByTitle('t1')).toBeInTheDocument();
    expect(screen.getByText('src/foo.ts')).toBeInTheDocument();
    expect(screen.getByText('Foo works')).toBeInTheDocument();
    expect(screen.getByText('npm test -- foo')).toBeInTheDocument();

    expect(screen.getByText('Second task')).toBeInTheDocument();
    // The blocked_by chip on the second card references the first card's id,
    // so 't1' renders twice (the id badge and the reference) — assert both.
    expect(screen.getByText('Blocked by').nextElementSibling).toHaveTextContent('t1');
    expect(screen.getAllByText('t1')).toHaveLength(2);

    expect(screen.queryByText('Original decomposition')).not.toBeInTheDocument();
    expect(screen.queryByText(/^Rework/)).not.toBeInTheDocument();
  });

  it('labels each cycle group when history plus the current cycle produce more than one group', () => {
    const plan: TaskPlan = {
      kind: 'rework',
      cycle: 1,
      history: [
        {
          cycle: 0,
          kind: 'greenfield',
          tasks: [{ id: 'orig-1', title: 'Original task', description: 'Original work' }],
        },
      ],
      tasks: [{ id: 'rw-1', title: 'Rework task', description: 'Fix the thing' }],
    };

    render(<TaskListArtifact plan={plan} />);

    expect(screen.getByText('Original decomposition')).toBeInTheDocument();
    expect(screen.getByText('Rework 1')).toBeInTheDocument();
    expect(screen.getByText('Original task')).toBeInTheDocument();
    expect(screen.getByText('Rework task')).toBeInTheDocument();
  });

  it('renders the notes callout instead of an empty-state placeholder for a rework cycle with no tasks', () => {
    const plan: TaskPlan = {
      kind: 'rework',
      cycle: 1,
      tasks: [],
      notes: 'No ticket was warranted — the prior cycle already covers this.',
    };

    render(<TaskListArtifact plan={plan} />);

    expect(
      screen.getByText('No ticket was warranted — the prior cycle already covers this.'),
    ).toBeInTheDocument();
    expect(screen.queryByText(/no tasks/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/empty/i)).not.toBeInTheDocument();
  });

  it('renders a warning banner without hiding the cards when findPlanIssues trips', () => {
    const plan: TaskPlan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [
        { id: 'dup', title: 'Task A', description: 'First', blocked_by: ['dup'] },
        { id: 'dup', title: 'Task B', description: 'Second' },
      ],
    };

    render(<TaskListArtifact plan={plan} />);

    expect(screen.getByText('Plan issues')).toBeInTheDocument();
    expect(screen.getByText('Duplicate task id: dup')).toBeInTheDocument();
    expect(screen.getByText('Task dup is blocked by itself')).toBeInTheDocument();

    expect(screen.getByText('Task A')).toBeInTheDocument();
    expect(screen.getByText('Task B')).toBeInTheDocument();
  });

  it('renders no warning banner for a clean plan', () => {
    const plan: TaskPlan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [{ id: 't1', title: 'Clean task', description: 'No issues here' }],
    };

    render(<TaskListArtifact plan={plan} />);

    expect(screen.queryByText('Plan issues')).not.toBeInTheDocument();
  });

  // The artifact is agent-written, not compiler-checked (types.ts:505), so a
  // malformed field must not throw regardless of what parseTaskPlan would
  // have caught upstream — these bypass it and construct the plan directly.
  it('does not throw when files is a scalar string instead of an array', () => {
    const plan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [{ id: 't1', title: 'Task', description: 'Do it', files: 'src/foo.ts' }],
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  it('does not throw when acceptance is a scalar string instead of an array', () => {
    const plan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [{ id: 't1', title: 'Task', description: 'Do it', acceptance: 'do the thing' }],
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  it('does not throw when blocked_by is a scalar string instead of an array', () => {
    const plan = {
      kind: 'greenfield',
      cycle: 0,
      tasks: [{ id: 't1', title: 'Task', description: 'Do it', blocked_by: 't1' }],
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  // The shape every `task-list.json` on disk actually has: tasks only. No
  // fixture above exercises it, which is how `undefined` reached a className.
  it('renders the production shape — tasks only, no kind or cycle', () => {
    const plan: TaskPlan = {
      tasks: [{ id: 't1', title: 'Only task', description: 'Do it' }],
    };

    const { container } = render(<TaskListArtifact plan={plan} />);

    expect(screen.getByText('Only task')).toBeInTheDocument();
    // A missing `kind` used to interpolate the string "undefined" as a class,
    // which no stylesheet defines and no gate in `npm run checks` reports.
    expect(container.innerHTML).not.toContain('undefined');
    expect(container.querySelector('.border-violet-500\\/50')).not.toBeNull();
    expect(screen.queryByText('Original decomposition')).not.toBeInTheDocument();
  });

  it('labels a history group with no cycle number as the original decomposition, never "Rework undefined"', () => {
    const plan = {
      tasks: [{ id: 'rw-1', title: 'Rework task', description: 'Fix it' }],
      history: [{ tasks: [{ id: 'orig-1', title: 'Original task', description: 'Original work' }] }],
    } as TaskPlan;

    render(<TaskListArtifact plan={plan} />);

    expect(screen.queryByText(/Rework undefined/)).not.toBeInTheDocument();
    expect(screen.getByText('Original task')).toBeInTheDocument();
    expect(screen.getByText('Rework task')).toBeInTheDocument();
  });

  it('does not throw when a history entry carries no tasks array', () => {
    const plan = {
      tasks: [{ id: 't1', title: 'Task', description: 'Do it' }],
      history: [{ cycle: 0 }],
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  it('does not throw when history is not an array', () => {
    const plan = {
      tasks: [{ id: 't1', title: 'Task', description: 'Do it' }],
      history: { cycle: 0 },
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  it('does not throw when notes is an object instead of a string', () => {
    const plan = {
      tasks: [{ id: 't1', title: 'Task', description: 'Do it' }],
      notes: { reason: 'nope' },
    } as unknown as TaskPlan;

    expect(() => render(<TaskListArtifact plan={plan} />)).not.toThrow();
  });

  it('does not emit an undefined class when kind is a string no palette entry matches', () => {
    // `isTaskPlan` now rejects this payload, so it can only arrive from a
    // caller that skipped the guard — the same bypass the scalar-field cases
    // above cover. The failure mode is silent by construction: an undefined
    // `Record` lookup renders as the literal class "undefined".
    const plan = {
      kind: 'Greenfield',
      cycle: 0,
      tasks: [{ id: 't1', title: 'Task', description: 'Do it' }],
    } as unknown as TaskPlan;

    const { container } = render(<TaskListArtifact plan={plan} />);

    expect(container.innerHTML).not.toContain('undefined');
    expect(container.querySelector('.border-violet-500\\/50')).not.toBeNull();
  });

  it('states that a zero-ticket plan with no notes produced nothing, rather than rendering blank', () => {
    const plan: TaskPlan = { tasks: [] };

    const { container } = render(<TaskListArtifact plan={plan} />);

    expect(screen.getByText(/produced no tickets/i)).toBeInTheDocument();
    expect(container.textContent?.trim()).not.toBe('');
  });
});
