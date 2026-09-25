/**
 * The two readings this block must never swap: a node that can still be
 * re-pointed, and one whose agent is already up.
 *
 * The status split is asserted against the same table
 * `domain::step_assignment::assignment_refusal` applies, because a surface
 * that offers a control the backend refuses — or withholds one it would have
 * accepted — is wrong in a way no type catches.
 *
 * `useStepAssignment` is the real hook in the payload tests rather than a
 * spy: the claim there is what reaches the typed wrapper, and a stubbed
 * `apply` would assert the stub's own argument list.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentCatalogEntry } from '../../../lib/agentCatalog';
import type { EffortLevel } from '../../../lib/effortLevels';
import type { AgentAvailability } from '../../../lib/featureDetail';
import type { RemoteRunMirror, StepOverride } from '../../../types';

const setStepAssignment = vi.fn<(input: unknown) => Promise<void>>();
const remoteSetStepAssignment = vi.fn<(input: unknown) => Promise<void>>();

const CATALOG: AgentCatalogEntry[] = [
  {
    kind: 'claude-code',
    display_label: 'claude-code',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'],
  },
  // AGENTS.md §2: hermes exposes effort only through its own config file, so
  // the capability is declared unsupported rather than reached for.
  {
    kind: 'hermes',
    display_label: 'hermes',
    lists_models: false,
    default_model: null,
    install_command: '',
    effort_levels: [],
  },
];

vi.mock('../../../lib/agentCatalog', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../../lib/agentCatalog')>()),
  useAgentCatalog: () => ({ agents: CATALOG, loading: false }),
}));

vi.mock('../../../lib/features', () => ({
  setStepAssignment: (input: unknown) => setStepAssignment(input),
  remoteSetStepAssignment: (input: unknown) => remoteSetStepAssignment(input),
}));

import { type HarnessOverrides } from '../../FeatureDetail/useHarnessOverrides';
import { useStepAssignment, type StepAssignment } from '../../FeatureDetail/useStepAssignment';
import { AssignmentSection } from './AssignmentSection';

const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'hermes', enabled: true, available: true },
  // Registered but not installed on the feature's machine: C-9 says it is
  // never offered, because the only thing choosing it can produce is a
  // failed spawn.
  { kind: 'codex', enabled: true, available: false },
];

const ASSIGNMENT: StepAssignment = {
  dirty: true,
  pinned: true,
  applying: false,
  error: null,
  clearError: vi.fn(),
  apply: vi.fn(async () => {}),
  reset: vi.fn(async () => {}),
};

function overridesOf(patch: Partial<HarnessOverrides> = {}): HarnessOverrides {
  return {
    machineAgents: MACHINE_AGENTS,
    availableModels: [{ value: 'sonnet', name: 'Sonnet' }],
    selectedModel: '',
    setSelectedModel: vi.fn(),
    isLoadingModels: false,
    availableAgents: ['claude-code', 'hermes'],
    selectedAgent: '',
    selectedEffort: '',
    setSelectedEffort: vi.fn(),
    seededEffort: '',
    featureAgentKind: 'opencode',
    inheritedAgentKind: 'opencode',
    retryEffortLevels: ['low', 'medium', 'high', 'xhigh', 'max'],
    onAgentChange: vi.fn(),
    adoptFeatureModel: vi.fn(),
    probeForFeature: vi.fn(),
    ...patch,
  };
}

function harnessSelect(): HTMLSelectElement | null {
  return screen.queryByLabelText('Harness') as HTMLSelectElement | null;
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('AssignmentSection', () => {
  it.each(['pending', 'awaiting_gate', 'failed', 'interrupted', 'completed'])(
    'offers the picker, Apply and Reset for a %s node',
    (status) => {
      render(
        <AssignmentSection status={status} overrides={overridesOf()} assignment={ASSIGNMENT} />,
      );

      expect(harnessSelect()).toBeInTheDocument();
      expect(screen.getByLabelText('Model')).toBeInTheDocument();
      expect(screen.getByLabelText('Effort')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Apply' })).toBeEnabled();
      expect(screen.getByRole('button', { name: /reset to inherited/i })).toBeInTheDocument();
      expect(screen.queryByText(/cannot be re-pointed mid-flight/i)).toBeNull();
    },
  );

  it.each(['running', 'verifying'])(
    'renders %s read-only, naming the status and the observed trio',
    (status) => {
      render(
        <AssignmentSection
          status={status}
          overrides={overridesOf()}
          assignment={ASSIGNMENT}
          observed={{ agentKind: 'claude-code', model: 'sonnet', effort: 'max' }}
        />,
      );

      expect(harnessSelect()).toBeNull();
      expect(screen.queryByRole('button', { name: 'Apply' })).toBeNull();
      expect(screen.queryByRole('button', { name: /reset to inherited/i })).toBeNull();
      expect(screen.getByText(/read-only/i)).toBeInTheDocument();
      expect(screen.getByText(new RegExp(`This node is ${status}`))).toBeInTheDocument();
      expect(screen.getByTitle('Harness')).toHaveTextContent(/^claude code$/);
      expect(screen.getByTitle('Model')).toHaveTextContent(/^sonnet$/);
      expect(screen.getByTitle('Effort')).toHaveTextContent(/^Max$/);
    },
  );

  it('renders what a running node spawned with, not an unapplied picker edit', () => {
    render(
      <AssignmentSection
        status="running"
        overrides={overridesOf({
          selectedAgent: 'claude-code',
          selectedModel: 'sonnet',
          selectedEffort: 'max',
        })}
        assignment={ASSIGNMENT}
        observed={{ agentKind: 'opencode', model: null, effort: 'low' }}
      />,
    );

    expect(screen.getByTitle('Harness')).toHaveTextContent(/^opencode$/);
    expect(screen.getByTitle('Model')).toHaveTextContent(/^Harness default$/);
    expect(screen.getByTitle('Effort')).toHaveTextContent(/^Low$/);
    expect(screen.queryByText(/claude code/i)).toBeNull();
    expect(screen.queryByText('sonnet')).toBeNull();
    expect(screen.queryByText(/^Max$/)).toBeNull();
    expect(screen.getByText(/was spawned with the assignment above/)).toBeInTheDocument();
  });

  it('claims nothing about a spawn the run has no evidence of', () => {
    render(
      <AssignmentSection
        status="running"
        overrides={overridesOf({ selectedAgent: 'claude-code' })}
        assignment={ASSIGNMENT}
      />,
    );

    expect(screen.queryByTitle('Harness')).toBeNull();
    expect(screen.queryByText(/was spawned with/)).toBeNull();
    expect(screen.getByText(/no launch evidence/i)).toBeInTheDocument();
    expect(screen.getByText(/cannot be re-pointed mid-flight/)).toBeInTheDocument();
  });

  it('draws no model chip for a spawn that claimed no model', () => {
    render(
      <AssignmentSection
        status="running"
        overrides={overridesOf()}
        assignment={ASSIGNMENT}
        observed={{ agentKind: 'opencode', effort: null }}
      />,
    );

    expect(screen.getByTitle('Harness')).toHaveTextContent(/^opencode$/);
    expect(screen.queryByTitle('Model')).toBeNull();
    expect(screen.getByTitle('Effort')).toHaveTextContent(/^No injected effort$/);
  });

  it('offers only the harnesses the machine can run, under an inherit placeholder', () => {
    render(
      <AssignmentSection status="pending" overrides={overridesOf()} assignment={ASSIGNMENT} />,
    );

    const options = Array.from(harnessSelect()!.options).map((o) => o.value);
    expect(options).toEqual(['', 'claude-code', 'hermes']);
    expect(harnessSelect()!.options[0]).toHaveTextContent('Inherit (opencode)');
  });

  it('names no harness for a node whose inherited one is not known here', () => {
    // No feature-wide harness: the node resolves through its workflow step
    // and the project default, which this surface cannot see. Naming the
    // `opencode` fallback would claim a harness the node may not run on, and
    // offer that harness's models under it.
    render(
      <AssignmentSection
        status="pending"
        overrides={overridesOf({ inheritedAgentKind: '' })}
        assignment={ASSIGNMENT}
      />,
    );

    expect(harnessSelect()!.options[0]).toHaveTextContent(/^Inherit$/);
    expect(screen.getByLabelText('Model')).toBeDisabled();
  });

  it('disables the effort control for a harness with no effort ladder', () => {
    render(
      <AssignmentSection
        status="pending"
        overrides={overridesOf({ selectedAgent: 'hermes', retryEffortLevels: [] })}
        assignment={ASSIGNMENT}
      />,
    );

    expect(screen.getByLabelText('Effort')).toBeDisabled();
  });

  it('holds Apply until the picker says something the node is not pinned to', () => {
    render(
      <AssignmentSection
        status="pending"
        overrides={overridesOf()}
        assignment={{ ...ASSIGNMENT, dirty: false }}
      />,
    );

    expect(screen.getByRole('button', { name: 'Apply' })).toBeDisabled();
  });

  it('holds Reset while the node has no pin', () => {
    render(
      <AssignmentSection
        status="pending"
        overrides={overridesOf()}
        assignment={{ ...ASSIGNMENT, pinned: false }}
      />,
    );

    expect(screen.getByRole('button', { name: /reset to inherited/i })).toBeDisabled();
  });

  it('offers Reset on a pinned node whose controls were blanked by hand', () => {
    // The picker reads "inherit" in every dimension, but the stored pin is
    // untouched until Apply — so the node is not yet inheriting.
    render(
      <AssignmentSection status="pending" overrides={overridesOf()} assignment={ASSIGNMENT} />,
    );

    expect(screen.getByRole('button', { name: /reset to inherited/i })).toBeEnabled();
  });

  it('surfaces a refused submit and dismisses it', () => {
    const clearError = vi.fn();
    render(
      <AssignmentSection
        status="pending"
        overrides={overridesOf()}
        assignment={{ ...ASSIGNMENT, error: 'Cannot change the assignment', clearError }}
      />,
    );

    expect(screen.getByRole('alert')).toHaveTextContent('Cannot change the assignment');
    fireEvent.click(screen.getByRole('button', { name: 'Dismiss assignment error' }));
    expect(clearError).toHaveBeenCalledTimes(1);
  });
});

/** A run the local app only mirrors: every write to it is the runner's. */
const DETACHED: RemoteRunMirror = {
  machine_id: 'm-rig',
  run_id: 'r-detached',
  project_id: 'p-demeteo',
  title: 'Editable per-step assignment',
  status: 'running',
  error: null,
  feature_id: 'f-ce92ca9f6ab23572',
  pr_url: null,
  pushed_branch: null,
  last_offset: 0,
  created_at: 0,
  updated_at: 0,
  last_notified_status: null,
};

const PINS: StepOverride[] = [
  { step_id: 's-implement', agent_kind: 'claude-code', model: 'sonnet', effort: 'max' },
];

function Mounted({
  status,
  remoteRun = null,
}: {
  status: string;
  remoteRun?: RemoteRunMirror | null;
}) {
  const [selectedAgent, setSelectedAgent] = useState('claude-code');
  const [selectedModel, setSelectedModel] = useState('sonnet');
  const [selectedEffort, setSelectedEffort] = useState<EffortLevel | ''>('max');
  const overrides = overridesOf({
    selectedAgent,
    selectedModel,
    selectedEffort,
    onAgentChange: setSelectedAgent,
    setSelectedModel,
    setSelectedEffort,
  });
  const assignment = useStepAssignment({
    overrides,
    selectedStepId: 's-implement',
    selectedExecutionId: 'se-implement',
    stepOverrides: PINS,
    remoteRun,
    reload: () => {},
  });
  return <AssignmentSection status={status} overrides={overrides} assignment={assignment} />;
}

describe('AssignmentSection submits', () => {
  it('sends the trio the picker shows when Apply is pressed', async () => {
    setStepAssignment.mockResolvedValue(undefined);
    render(<Mounted status="pending" />);

    fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'low' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    expect(setStepAssignment).toHaveBeenCalledWith({
      stepExecutionId: 'se-implement',
      agentKind: 'claude-code',
      model: 'sonnet',
      effort: 'low',
    });
  });

  it('sends all three null when Reset to inherited is pressed', async () => {
    setStepAssignment.mockResolvedValue(undefined);
    render(<Mounted status="pending" />);

    fireEvent.click(screen.getByRole('button', { name: /reset to inherited/i }));

    expect(setStepAssignment).toHaveBeenCalledWith({
      stepExecutionId: 'se-implement',
      agentKind: null,
      model: null,
      effort: null,
    });
    expect(remoteSetStepAssignment).not.toHaveBeenCalled();
  });

  it('routes a detached run to the runner, addressed by machine and run', async () => {
    remoteSetStepAssignment.mockResolvedValue(undefined);
    render(<Mounted status="pending" remoteRun={DETACHED} />);

    fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'low' } });
    fireEvent.click(await screen.findByRole('button', { name: 'Apply' }));

    expect(remoteSetStepAssignment).toHaveBeenCalledWith({
      machineId: 'm-rig',
      runId: 'r-detached',
      stepExecutionId: 'se-implement',
      agentKind: 'claude-code',
      model: 'sonnet',
      effort: 'low',
    });
    // Not both: the shadow row the local command would write is the runner's
    // to own, and a second write behind its back is how the two copies drift.
    expect(setStepAssignment).not.toHaveBeenCalled();
  });
});
