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

/**
 * The markup with each icon's vector collapsed to its class. The paths are
 * lucide's to redraw, so pinning them would fail this on a dependency bump
 * rather than on a change to what this component decides.
 */
function shape(container: HTMLElement): string {
  const clone = container.cloneNode(true) as HTMLElement;
  for (const svg of Array.from(clone.querySelectorAll('svg'))) {
    const icon = clone.ownerDocument.createElement('icon');
    icon.setAttribute('class', svg.getAttribute('class') ?? '');
    svg.replaceWith(icon);
  }
  return clone.innerHTML;
}

/**
 * What an executed step drew before the planned variant existed, captured from
 * that implementation. The planned rendering shares this component, and a chip
 * that quietly restyles or renames an observed reading is the failure C-10
 * names: it would claim a run spawned something it did not.
 */
const OBSERVED_SHAPE =
  '<span role="img" aria-label="Actual assignment for Implement: Agent: codex; Model: gpt-5.6-codex; Effective effort: High" class="flex min-w-0 flex-nowrap items-center gap-1 font-mono text-[9px] "><span class="max-w-[160px] flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300" title="Agent: codex"><icon class="lucide lucide-bot h-2.5 w-2.5 shrink-0 text-slate-400"></icon><span class="truncate">codex</span></span><span class="grow basis-0 max-w-max overflow-hidden flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300" title="Model: gpt-5.6-codex"><icon class="lucide lucide-zap h-2.5 w-2.5 shrink-0 text-slate-400"></icon><span class="max-w-[132px] truncate">gpt-5.6-codex</span></span><span class="max-w-[160px] flex min-w-0 items-center gap-1 rounded border border-slate-600/40 bg-slate-700/20 px-1.5 py-0.5 text-slate-300" title="Effective effort: High"><icon class="lucide lucide-gauge h-2.5 w-2.5 shrink-0 text-slate-400"></icon><span class="truncate">High</span></span></span>';

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
  it('draws the pinned trio where a node with no spawn evidence drew nothing', () => {
    const unrun = render(<AssignmentChips subject="Implement" />);
    expect(unrun.container).toBeEmptyDOMElement();
    unrun.unmount();

    render(
      <AssignmentChips
        subject="Implement"
        variant="planned"
        agentKind="codex"
        model="gpt-5.6-codex"
        effort="high"
      />,
    );

    const [agent, model, effort, ...rest] = chips();
    expect(rest).toHaveLength(0);
    expect(agent).toHaveAttribute('title', 'Agent (planned): codex');
    expect(agent).toHaveTextContent(/^codex$/);
    expect(model).toHaveAttribute('title', 'Model (planned): gpt-5.6-codex');
    expect(effort).toHaveAttribute('title', 'Effort (planned): High');
    expect(group()).toHaveAccessibleName(
      'Planned assignment for Implement: Agent: codex; Model: gpt-5.6-codex; Effort: High',
    );
  });

  it('marks the planned chips as not-yet-observed rather than restyling nothing', () => {
    const observed = render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="high" />,
    );
    const observedChip = observed.getByTitle('Agent: codex');
    expect(observedChip).not.toHaveClass('border-dashed');
    observed.unmount();

    render(<AssignmentChips subject="Implement" variant="planned" agentKind="codex" />);

    expect(screen.getByTitle('Agent (planned): codex')).toHaveClass('border-dashed');
  });

  it('leaves an executed step\'s markup and accessible name byte-identical', () => {
    const { container } = render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="high" />,
    );

    expect(shape(container)).toBe(OBSERVED_SHAPE);
  });

  it('names the planned reading differently from the observed one', () => {
    const observed = render(
      <AssignmentChips subject="Implement" agentKind="codex" model="gpt-5.6-codex" effort="high" />,
    );
    const observedName = observed.getByRole('img').getAttribute('aria-label');
    observed.unmount();

    render(
      <AssignmentChips
        subject="Implement"
        variant="planned"
        agentKind="codex"
        model="gpt-5.6-codex"
        effort="high"
      />,
    );
    const plannedName = group().getAttribute('aria-label');

    expect(plannedName).not.toBe(observedName);
    expect(plannedName).not.toMatch(/Actual/);
    expect(observedName).not.toMatch(/Planned/);
  });

  it('draws only the dimensions the pin actually set', () => {
    render(<AssignmentChips subject="Implement" variant="planned" agentKind="codex" model={null} />);

    expect(chips()).toHaveLength(1);
    expect(screen.queryByTitle(/model/i)).toBeNull();
    expect(group()).toHaveAccessibleName('Planned assignment for Implement: Agent: codex');
  });

  it.each([
    ['nothing pinned', {}],
    ['every dimension left to inherit', { agentKind: null, model: null, effort: null }],
    ['a blank agent and nothing else', { agentKind: '   ' }],
  ])('renders nothing for a planned assignment with %s', (_case, props) => {
    const { container } = render(
      <AssignmentChips subject="Implement" variant="planned" {...props} />,
    );

    expect(container).toBeEmptyDOMElement();
  });
});
