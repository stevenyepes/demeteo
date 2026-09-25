/**
 * Apply and Reset for one node, and the two readings they must never swap.
 *
 * A pin that did not land, reported as landed, is the failure worth a test
 * here: the picker goes on showing the chosen trio either way, so nothing on
 * screen distinguishes a refused write from an applied one except the error
 * the hook is required to keep.
 *
 * `reconcileEffort` and `effortLevelsFor` are the real ones, as in
 * `useHarnessOverrides.test.ts` — the clamp claims are about the ladder hermes
 * actually declares, and a stub would assert the stub.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentCatalogEntry } from '../../lib/agentCatalog';
import type { AgentAvailability } from '../../lib/featureDetail';
import type { Project, RemoteRunMirror, StepOverride } from '../../types';

const getAgentModels = vi.fn<(machineId: string, agentKind: string) => Promise<unknown>>();
const getProjectById = vi.fn<(projectId: string) => Promise<Partial<Project> | null>>();
const listAgentConfigs = vi.fn<(input: unknown) => Promise<AgentAvailability[]>>();
const setStepAssignment = vi.fn<(input: unknown) => Promise<void>>();
const remoteSetStepAssignment = vi.fn<(input: unknown) => Promise<void>>();
const reload = vi.fn();

const CATALOG: AgentCatalogEntry[] = [
  {
    kind: 'opencode',
    display_label: 'opencode',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'],
  },
  {
    kind: 'claude-code',
    display_label: 'claude-code',
    lists_models: true,
    default_model: null,
    install_command: '',
    effort_levels: ['low', 'medium', 'high', 'xhigh', 'max'],
  },
  // AGENTS.md §2: hermes exposes effort only through its own config file, so
  // Demeteo declares the capability unsupported rather than reaching into it.
  {
    kind: 'hermes',
    display_label: 'hermes',
    lists_models: false,
    default_model: null,
    install_command: '',
    effort_levels: [],
  },
];

// Swapped mid-test by the one case whose claim is about the catalog landing
// after the seed; every other case sees `CATALOG` from the first render.
let catalog: AgentCatalogEntry[] = CATALOG;

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: (machineId: string, agentKind: string) => getAgentModels(machineId, agentKind),
}));

vi.mock('../../lib/featureDetail', () => ({
  getProjectById: (projectId: string) => getProjectById(projectId),
  listAgentConfigs: (input: unknown) => listAgentConfigs(input),
}));

vi.mock('../../lib/agentCatalog', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/agentCatalog')>()),
  useAgentCatalog: () => ({ agents: catalog, loading: false }),
}));

vi.mock('../../lib/features', () => ({
  setStepAssignment: (input: unknown) => setStepAssignment(input),
  remoteSetStepAssignment: (input: unknown) => remoteSetStepAssignment(input),
}));

import { useHarnessOverrides, type HarnessOverrides } from './useHarnessOverrides';
import { useStepAssignment, type StepAssignment } from './useStepAssignment';

const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'opencode', enabled: true, available: true },
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'hermes', enabled: true, available: true },
];

const PINS: StepOverride[] = [
  { step_id: 's-implement', agent_kind: 'claude-code', model: 'sonnet', effort: 'max' },
  // Pinned to a harness with no effort ladder at all: the stored level is
  // unreachable in the control, which must not read as an unsaved edit.
  { step_id: 's-review', agent_kind: 'hermes', model: null, effort: 'high' },
];

const REMOTE_RUN = {
  machine_id: 'm-7',
  run_id: 'r-42',
} as RemoteRunMirror;

interface Props {
  selectedStepId: string | null;
  selectedExecutionId: string | null;
  remoteRun: RemoteRunMirror | null;
}

interface Mounted {
  overrides: HarnessOverrides;
  assignment: StepAssignment;
}

function mount(initial: Partial<Props> = {}) {
  const initialProps: Props = {
    selectedStepId: 's-implement',
    selectedExecutionId: 'se-implement',
    remoteRun: null,
    ...initial,
  };
  return renderHook(
    (props: Props): Mounted => {
      // A fresh `step_overrides` array every render is what the real caller
      // hands down — the feature is re-read on every poll — so any identity
      // claim below has to survive it.
      const overrides = useHarnessOverrides({
        selectedStepId: props.selectedStepId,
        stepOverrides: [...PINS],
      });
      const assignment = useStepAssignment({
        overrides,
        selectedStepId: props.selectedStepId,
        selectedExecutionId: props.selectedExecutionId,
        stepOverrides: [...PINS],
        remoteRun: props.remoteRun,
        reload,
      });
      return { overrides, assignment };
    },
    { initialProps },
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  catalog = CATALOG;
  getAgentModels.mockResolvedValue([{ value: 'm-1', name: 'm-1' }]);
  getProjectById.mockResolvedValue({ id: 'p-1', remote_host: null });
  listAgentConfigs.mockResolvedValue(MACHINE_AGENTS);
  setStepAssignment.mockResolvedValue(undefined);
  remoteSetStepAssignment.mockResolvedValue(undefined);
});

describe('useStepAssignment', () => {
  it('opens on the node\'s own pin with nothing to apply', async () => {
    const { result } = mount();

    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));
    expect(result.current.overrides.selectedModel).toBe('sonnet');
    expect(result.current.overrides.selectedEffort).toBe('max');
    expect(result.current.assignment.dirty).toBe(false);
  });

  it('is clean for a pin whose harness cannot run the stored effort', async () => {
    const { result } = mount({ selectedStepId: 's-review', selectedExecutionId: 'se-review' });

    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('hermes'));
    // Seeded as "inherit" because hermes declares no ladder; the user has
    // still edited nothing.
    expect(result.current.overrides.selectedEffort).toBe('');
    expect(result.current.assignment.dirty).toBe(false);
  });

  it('clears and goes clean when the selection moves to an unpinned node', async () => {
    const { result, rerender } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    rerender({ selectedStepId: 's-research', selectedExecutionId: 'se-research', remoteRun: null });

    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe(''));
    expect(result.current.overrides.selectedModel).toBe('');
    expect(result.current.overrides.selectedEffort).toBe('');
    expect(result.current.assignment.dirty).toBe(false);
  });

  it('goes dirty once the picker leaves the pin, and clean again when it returns', async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedModel).toBe('sonnet'));

    act(() => { result.current.overrides.setSelectedModel('opus'); });
    await waitFor(() => expect(result.current.assignment.dirty).toBe(true));

    act(() => { result.current.overrides.setSelectedModel('sonnet'); });
    await waitFor(() => expect(result.current.assignment.dirty).toBe(false));
  });

  it('stays clean when the catalog lands after the seed with a shorter ladder', async () => {
    // A harness this build has no static ladder for is seeded against the full
    // fallback; the catalog then declares a shorter one. Re-reading the pin
    // against the new ladder would call the untouched node edited — and hold
    // Retry and Replay shut on it.
    catalog = CATALOG.filter((a) => a.kind !== 'claude-code');
    const { result, rerender } = mount();
    await waitFor(() => expect(result.current.overrides.selectedEffort).toBe('max'));

    catalog = [CATALOG[0], { ...CATALOG[1], effort_levels: ['low'] }, CATALOG[2]];
    rerender({ selectedStepId: 's-implement', selectedExecutionId: 'se-implement', remoteRun: null });

    expect(result.current.overrides.selectedEffort).toBe('max');
    expect(result.current.assignment.dirty).toBe(false);
  });

  it('reconciles the effort through reconcileEffort when the harness changes', async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedEffort).toBe('max'));

    act(() => { result.current.overrides.onAgentChange('hermes'); });

    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('hermes'));
    expect(result.current.overrides.selectedEffort).toBe('');
    expect(result.current.overrides.retryEffortLevels).toEqual([]);
    expect(result.current.assignment.dirty).toBe(true);
  });

  it('applies a local run\'s trio through setStepAssignment', async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedModel).toBe('sonnet'));

    act(() => { result.current.overrides.setSelectedModel('opus'); });
    await act(async () => { await result.current.assignment.apply(); });

    expect(remoteSetStepAssignment).not.toHaveBeenCalled();
    expect(setStepAssignment).toHaveBeenCalledWith({
      stepExecutionId: 'se-implement',
      agentKind: 'claude-code',
      model: 'opus',
      effort: 'max',
    });
    expect(reload).toHaveBeenCalled();
    expect(result.current.assignment.error).toBeNull();
  });

  it('routes a detached run\'s apply to the runner that owns it', async () => {
    const { result } = mount({ remoteRun: REMOTE_RUN });
    await waitFor(() => expect(result.current.overrides.selectedModel).toBe('sonnet'));

    await act(async () => { await result.current.assignment.apply(); });

    expect(setStepAssignment).not.toHaveBeenCalled();
    expect(remoteSetStepAssignment).toHaveBeenCalledWith({
      machineId: 'm-7',
      runId: 'r-42',
      stepExecutionId: 'se-implement',
      agentKind: 'claude-code',
      model: 'sonnet',
      effort: 'max',
    });
  });

  it('resets by submitting all three null and emptying the picker', async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.reset(); });

    expect(setStepAssignment).toHaveBeenCalledWith({
      stepExecutionId: 'se-implement',
      agentKind: null,
      model: null,
      effort: null,
    });
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe(''));
    expect(result.current.overrides.selectedModel).toBe('');
    expect(result.current.overrides.selectedEffort).toBe('');
  });

  it('says why Apply did nothing for a node with no execution', async () => {
    const { result } = mount({ selectedExecutionId: null });
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.apply(); });

    expect(setStepAssignment).not.toHaveBeenCalled();
    expect(result.current.assignment.error).toMatch(/no execution/);
    expect(result.current.assignment.applying).toBe(false);
  });

  it('surfaces a refusal as the backend\'s own text, not as a silent success', async () => {
    const refusal = "Cannot change the assignment of a step in 'running' status";
    setStepAssignment.mockRejectedValue({ kind: 'validation', message: refusal });
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.apply(); });

    expect(result.current.assignment.error).toBe(refusal);
    expect(reload).not.toHaveBeenCalled();

    act(() => { result.current.assignment.clearError(); });
    expect(result.current.assignment.error).toBeNull();
  });

  it('surfaces an older runner\'s unknown method as a failure', async () => {
    const unknown = 'unknown method: set_step_assignment';
    remoteSetStepAssignment.mockRejectedValue(new Error(unknown));
    const { result } = mount({ remoteRun: REMOTE_RUN });
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.apply(); });

    expect(result.current.assignment.error).toBe(unknown);
    expect(reload).not.toHaveBeenCalled();
  });

  it('leaves the picker pinned when a reset is refused', async () => {
    setStepAssignment.mockRejectedValue(new Error('write failed'));
    const { result } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.reset(); });

    expect(result.current.assignment.error).toBe('write failed');
    expect(result.current.overrides.selectedAgent).toBe('claude-code');
    expect(result.current.overrides.selectedModel).toBe('sonnet');
  });

  it('drops a refusal when the inspector retargets to another node', async () => {
    setStepAssignment.mockRejectedValue({
      kind: 'validation',
      message: "Cannot change the assignment of a step in 'running' status",
    });
    const { result, rerender } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    await act(async () => { await result.current.assignment.apply(); });
    expect(result.current.assignment.error).not.toBeNull();

    rerender({ selectedStepId: 's-research', selectedExecutionId: 'se-research', remoteRun: null });

    await waitFor(() => expect(result.current.assignment.error).toBeNull());
  });

  it('leaves a submit that outlived its node attached to nothing', async () => {
    let refuse: (err: unknown) => void = () => {};
    setStepAssignment.mockReturnValue(new Promise<void>((_, reject) => { refuse = reject; }));
    const { result, rerender } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    let pending: Promise<void> | undefined;
    act(() => { pending = result.current.assignment.apply(); });
    expect(result.current.assignment.applying).toBe(true);

    rerender({ selectedStepId: 's-research', selectedExecutionId: 'se-research', remoteRun: null });
    expect(result.current.assignment.applying).toBe(false);

    await act(async () => {
      refuse({ kind: 'validation', message: "Cannot change the assignment of a step in 'running' status" });
      await pending;
    });

    expect(result.current.assignment.applying).toBe(false);
    expect(result.current.assignment.error).toBeNull();
  });

  it('keeps every member identity-stable across a render that changed none', async () => {
    const { result, rerender } = mount();
    await waitFor(() => expect(result.current.overrides.selectedAgent).toBe('claude-code'));

    const before = result.current.assignment;
    rerender({ selectedStepId: 's-implement', selectedExecutionId: 'se-implement', remoteRun: null });
    const after = result.current.assignment;

    expect(after).toBe(before);
    for (const key of Object.keys(before) as Array<keyof StepAssignment>) {
      expect(after[key]).toBe(before[key]);
    }
  });
});
