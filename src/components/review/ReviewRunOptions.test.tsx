// The assertions are about what an *untouched* control is worth. Every one of
// these four selects renders `''` until the user picks, and `''` is what
// `RunChoice` reads as inherit — so a default substituted here, however
// sensible, is a pin the user never asked for. The availability case is mounted
// through the real `useRunChoice`: the filter that keeps a disabled or
// uninstalled harness out of the list is the hook's, and a stub of it would
// assert only that this file can type a shorter array.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { useRunChoice, type RunChoiceState } from '../../hooks/useRunChoice';
import type { AgentAvailability } from '../../lib/featureDetail';
import type { WorkflowWithSteps } from '../../types';
import { ReviewRunOptions } from './ReviewRunOptions';

vi.mock('../../lib/agentModels', () => ({ getAgentModels: vi.fn().mockResolvedValue([]) }));
vi.mock('../../lib/errorBus', () => ({
  useErrorBus: () => ({ toasts: [], reportError: vi.fn(), dismiss: vi.fn(), clear: vi.fn() }),
}));

const CATALOG = [
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high'] },
  { kind: 'codex', display_label: 'Codex', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high'] },
  { kind: 'opencode', display_label: 'opencode', lists_models: true, default_model: null, install_command: '', effort_levels: ['low', 'medium', 'high'] },
  { kind: 'pi', display_label: 'Pi', lists_models: true, default_model: null, install_command: '', effort_levels: [] },
];

/** One machine's answer: two usable, one switched off in settings, one never
 *  installed. */
const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'codex', enabled: true, available: true },
  { kind: 'opencode', enabled: false, available: true },
  { kind: 'pi', enabled: true, available: false },
];

function workflow(id: string, name: string): WorkflowWithSteps {
  return {
    id,
    name,
    description: '',
    is_starter: false,
    created_at: 0,
    updated_at: 0,
    steps: [],
    version: 1,
    version_id: `${id}-v1`,
  };
}

const WORKFLOWS = [workflow('wf-1', 'Code Review'), workflow('wf-2', 'Deep Review')];

function stubChoice(overrides: Partial<RunChoiceState> = {}): RunChoiceState {
  return {
    workflowId: '',
    setWorkflowId: vi.fn(),
    agentKind: '',
    setAgentKind: vi.fn(),
    model: '',
    setModel: vi.fn(),
    effort: '',
    setEffort: vi.fn(),
    availableAgents: ['claude-code', 'codex'],
    availableModels: [],
    isLoadingModels: false,
    effortLevels: ['low', 'medium', 'high'],
    choice: { workflowId: undefined, agentKind: undefined, model: undefined, effort: undefined },
    ...overrides,
  };
}

/** The component under test driven by the hook that feeds it in production, so
 *  the availability contract is asserted where it is actually decided. */
function Host({ machineAgents }: { machineAgents: AgentAvailability[] }) {
  const choice = useRunChoice({ machineAgents, machineId: 'm-1' });
  return <ReviewRunOptions choice={choice} workflows={WORKFLOWS} />;
}

function optionTexts(select: HTMLElement): string[] {
  return within(select)
    .getAllByRole('option')
    .map((o) => o.textContent ?? '');
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((cmd: string) =>
    cmd === 'list_agents'
      ? Promise.resolve(CATALOG)
      : Promise.reject(new Error(`unexpected command ${cmd}`)),
  );
});

describe('ReviewRunOptions', () => {
  it('leaves every control on its placeholder, which is the inherit value', () => {
    render(<ReviewRunOptions choice={stubChoice()} workflows={WORKFLOWS} />);

    for (const label of ['Workflow', 'Harness', 'Model', 'Effort']) {
      expect(screen.getByLabelText(label)).toHaveValue('');
    }
    expect(screen.getByLabelText<HTMLSelectElement>('Workflow').selectedOptions[0]).toHaveValue('');
  });

  it('names the placeholders as defaults rather than leaving them blank', () => {
    render(<ReviewRunOptions choice={stubChoice()} workflows={WORKFLOWS} />);

    expect(optionTexts(screen.getByLabelText('Workflow'))[0]).toMatch(/default/i);
    expect(optionTexts(screen.getByLabelText('Harness'))[0]).toMatch(/default/i);
  });

  // A chosen harness does not inherit a model: the project's belongs to the
  // project's harness, so a placeholder reading "default" would promise one.
  it('does not offer the model placeholder as a default once a harness is chosen', () => {
    render(
      <ReviewRunOptions
        choice={stubChoice({ agentKind: 'codex', choice: { agentKind: 'codex' } })}
        workflows={WORKFLOWS}
      />,
    );

    expect(optionTexts(screen.getByLabelText('Model'))[0]).not.toMatch(/default/i);
  });

  it('offers every workflow it is given', () => {
    render(<ReviewRunOptions choice={stubChoice()} workflows={WORKFLOWS} />);

    expect(optionTexts(screen.getByLabelText('Workflow'))).toEqual([
      expect.stringMatching(/default/i),
      'Code Review',
      'Deep Review',
    ]);
  });

  it('reports the chosen workflow id to its owner', async () => {
    const setWorkflowId = vi.fn();
    render(<ReviewRunOptions choice={stubChoice({ setWorkflowId })} workflows={WORKFLOWS} />);

    await userEvent.selectOptions(screen.getByLabelText('Workflow'), 'wf-2');

    expect(setWorkflowId).toHaveBeenCalledWith('wf-2');
  });

  it('does not offer a harness the machine reported disabled or uninstalled', async () => {
    render(<Host machineAgents={MACHINE_AGENTS} />);

    await waitFor(() =>
      expect(optionTexts(screen.getByLabelText('Harness'))).toEqual([
        expect.stringMatching(/default/i),
        'claude code',
        'codex',
      ]),
    );
  });
});
