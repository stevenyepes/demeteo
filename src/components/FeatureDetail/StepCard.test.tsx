import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { densityClasses } from '../../lib/density';
import type { EffortLevel } from '../../lib/effortLevels';
import { NO_INJECTED_EFFORT_LABEL } from '../../lib/runEventAssignments';
import type { StepExecution } from '../../types';
import { StepCard } from './StepCard';

const step = (over: Partial<StepExecution> = {}): StepExecution => ({
  id: 'se-implement',
  feature_id: 'feature-1',
  step_id: 's-implement',
  step_index: 0,
  step_kind: 'agent',
  status: 'running',
  artifact_paths: [],
  created_at: 0,
  updated_at: 0,
  ...over,
});

function renderCard({
  execution = step(),
  agentKind,
  effort,
}: {
  execution?: StepExecution;
  agentKind?: string | null;
  effort?: EffortLevel | null;
} = {}) {
  return render(
    <StepCard
      step={execution}
      index={0}
      isActiveGate={false}
      isSelected={false}
      cardRef={() => {}}
      density={densityClasses('comfortable')}
      onSelect={() => {}}
      onDecideGate={() => {}}
      agentKind={agentKind}
      effort={effort}
    />,
  );
}

describe('StepCard observed assignment', () => {
  it.each([
    ['high', 'High'],
    ['xhigh', 'Extra high'],
  ] as const)('shows the observed agent and %s effort label', (effort, label) => {
    renderCard({ agentKind: 'codex', effort });

    expect(screen.getByTitle('Agent: codex')).toHaveTextContent('codex');
    expect(screen.getByTitle(`Effective effort: ${label}`)).toHaveTextContent(label);
    if (effort === 'xhigh') expect(screen.queryByText('Max')).not.toBeInTheDocument();
  });

  it('distinguishes explicit null effort from absent spawn evidence', () => {
    const { rerender } = renderCard({ agentKind: 'hermes', effort: null });

    expect(screen.getByTitle('Agent: hermes')).toBeInTheDocument();
    expect(
      screen.getByTitle(`Effective effort: ${NO_INJECTED_EFFORT_LABEL}`),
    ).toHaveTextContent(NO_INJECTED_EFFORT_LABEL);

    rerender(
      <StepCard
        step={step()}
        index={0}
        isActiveGate={false}
        isSelected={false}
        cardRef={() => {}}
        density={densityClasses('comfortable')}
        onSelect={() => {}}
        onDecideGate={() => {}}
      />,
    );

    expect(screen.queryByText('Agent', { selector: 'span' })).not.toBeInTheDocument();
    expect(screen.queryByText('Effort', { selector: 'span' })).not.toBeInTheDocument();
  });

  it('does not show assignment metadata for a gate without spawn evidence', () => {
    renderCard({
      execution: step({ step_id: 's-review', step_kind: 'gate', status: 'awaiting_gate' }),
    });

    expect(screen.queryByText('Agent', { selector: 'span' })).not.toBeInTheDocument();
    expect(screen.queryByText('Effort', { selector: 'span' })).not.toBeInTheDocument();
  });

  it('keeps a long observed agent value available through its full title', () => {
    const longAgent = 'company-agent-provider-with-a-very-long-observed-runtime-name';
    renderCard({ agentKind: longAgent, effort: 'medium' });

    const badge = screen.getByTitle(`Agent: ${longAgent}`);
    expect(badge).toHaveAccessibleName(`Agent: ${longAgent}`);
    expect(badge).toHaveTextContent(longAgent);
  });

  it('remains memoized while assignment props stay primitive', () => {
    expect(StepCard).toHaveProperty('$$typeof', Symbol.for('react.memo'));
  });
});
