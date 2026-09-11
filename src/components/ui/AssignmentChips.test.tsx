import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { HARNESS_DEFAULT_MODEL_LABEL } from '../../lib/runEventAssignments';
import { AssignmentChips } from './AssignmentChips';

function group(): HTMLElement {
  return screen.getByRole('img');
}

function chips(): HTMLElement[] {
  return Array.from(group().children) as HTMLElement[];
}

describe('AssignmentChips', () => {
  it('draws agent, model and effort in that order with titles and value text', () => {
    render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="xhigh" />,
    );

    const [agent, model, effort, ...rest] = chips();
    expect(rest).toHaveLength(0);
    expect(agent).toHaveAttribute('title', 'Agent: codex');
    expect(agent).toHaveTextContent(/^codex$/);
    expect(model).toHaveAttribute('title', 'Model: gpt-5.6-codex');
    expect(model).toHaveTextContent(/^gpt-5\.6-codex$/);
    expect(effort).toHaveAttribute('title', 'Effective effort: Extra high');
    expect(effort).toHaveTextContent(/^Extra high$/);
  });

  it('shows the harness default when no tier pinned a model', () => {
    render(<AssignmentChips subject="Implement" agentKind="hermes" model={null} effort={null} />);

    expect(HARNESS_DEFAULT_MODEL_LABEL).toBe('Harness default');
    expect(screen.getByTitle('Model: Harness default')).toHaveTextContent(/^Harness default$/);
    expect(group()).toHaveAccessibleName(
      'Actual assignment for Implement: Agent: hermes; Model: Harness default; Effective effort: No injected effort',
    );
  });

  it('draws no model chip and no model segment when the spawn made no model claim', () => {
    const { container } = render(
      <AssignmentChips subject="Implement" agentKind="codex" effort="high" />,
    );

    expect(container.querySelector('[title^="Model:"]')).toBeNull();
    expect(chips()).toHaveLength(2);
    expect(screen.getByTitle('Agent: codex')).toBeInTheDocument();
    expect(screen.getByTitle('Effective effort: High')).toBeInTheDocument();
    expect(group()).toHaveAccessibleName(
      'Actual assignment for Implement: Agent: codex; Effective effort: High',
    );
  });

  it.each([
    ['no agent', { agentKind: undefined, effort: 'high' as const }],
    ['a null agent', { agentKind: null, effort: 'high' as const }],
    ['a blank agent', { agentKind: '   ', effort: 'high' as const }],
    ['no effort evidence', { agentKind: 'codex', effort: undefined }],
  ])('renders nothing for %s, even with a model', (_case, props) => {
    const { container } = render(
      <AssignmentChips subject="Implement" model="gpt-5.6-codex" {...props} />,
    );

    expect(container).toBeEmptyDOMElement();
  });

  it('announces the trio once, on a single img', () => {
    render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="low" />,
    );

    expect(screen.getAllByRole('img')).toHaveLength(1);
    expect(group()).toHaveAccessibleName(
      'Actual assignment for Implement: Agent: codex; Model: gpt-5.6-codex; Effective effort: Low',
    );
    expect(group().querySelectorAll('[role], [aria-label]')).toHaveLength(0);
  });

  it('marks each value with an aria-hidden icon instead of a prefix word', () => {
    render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="low" />,
    );

    expect(screen.queryByText('Agent')).not.toBeInTheDocument();
    expect(screen.queryByText('Effort')).not.toBeInTheDocument();
    expect(screen.queryByText('Model')).not.toBeInTheDocument();
    for (const chip of chips()) {
      const icon = chip.firstElementChild;
      expect(icon?.tagName.toLowerCase()).toBe('svg');
      expect(icon).toHaveAttribute('aria-hidden', 'true');
    }
  });

  it('stays on one line with every value truncating inside a chip that may shrink', () => {
    render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="low" />,
    );

    expect(group()).toHaveClass('flex-nowrap');
    expect(group()).not.toHaveClass('flex-wrap');
    const all = chips();
    expect(all).toHaveLength(3);
    for (const chip of all) {
      expect(chip).toHaveClass('min-w-0');
      expect(chip.querySelector('svg')).toHaveClass('shrink-0');
      expect(chip.querySelector('span')).toHaveClass('truncate');
    }
  });

  it('takes a short row out of the model first, leaving agent and effort whole', () => {
    render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="low" />,
    );

    const [agent, model, effort] = chips();
    expect(model).toHaveClass('basis-0', 'grow', 'max-w-max', 'overflow-hidden');
    expect(model.querySelector('span')).toHaveClass('max-w-[132px]');
    for (const sibling of [agent, effort]) {
      expect(sibling).not.toHaveClass('basis-0');
      expect(sibling).not.toHaveClass('grow');
      expect(sibling).toHaveClass('max-w-[160px]');
    }
  });

  it('keeps a long model id whole in the title and the accessible name', () => {
    const longModel = 'anthropic/claude-opus-5-20260601-extended-context-preview-build-7';
    expect(longModel.length).toBeGreaterThan(60);

    render(<AssignmentChips subject="Implement" agentKind="codex" model={longModel} effort="high" />);

    expect(screen.getByTitle(`Model: ${longModel}`)).toHaveTextContent(longModel);
    expect(group()).toHaveAccessibleName(
      `Actual assignment for Implement: Agent: codex; Model: ${longModel}; Effective effort: High`,
    );
  });
});
