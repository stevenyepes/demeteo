/**
 * One picker, addressed at one node at a time.
 *
 * The failure these pin is not a wrong value but a *stale* one: a selection
 * made for the node the user was reading, still standing in the control after
 * they moved to another node, is indistinguishable from a choice they made for
 * the node now in front of them — and Apply would pin it there.
 *
 * `effortLevelsFor` and `reconcileEffort` are deliberately the real ones. The
 * claim in `clamps a seeded effort the pinned harness cannot run` is about the
 * ladder hermes actually declares (empty, by decision), so a stubbed clamp
 * would assert the stub.
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentCatalogEntry } from '../../lib/agentCatalog';
import type { AgentAvailability } from '../../lib/featureDetail';
import type { Project, StepOverride } from '../../types';

const getAgentModels = vi.fn<(machineId: string, agentKind: string) => Promise<unknown>>();
const getProjectById = vi.fn<(projectId: string) => Promise<Partial<Project> | null>>();
const listAgentConfigs = vi.fn<(input: unknown) => Promise<AgentAvailability[]>>();

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
  // The honest-degradation case: no per-invocation effort control at all, and
  // §2 forbids reaching into `$HERMES_HOME/config.yaml` to invent one.
  {
    kind: 'hermes',
    display_label: 'hermes',
    lists_models: false,
    default_model: null,
    install_command: '',
    effort_levels: [],
  },
];

vi.mock('../../lib/agentModels', () => ({
  getAgentModels: (machineId: string, agentKind: string) => getAgentModels(machineId, agentKind),
}));

vi.mock('../../lib/featureDetail', () => ({
  getProjectById: (projectId: string) => getProjectById(projectId),
  listAgentConfigs: (input: unknown) => listAgentConfigs(input),
}));

vi.mock('../../lib/agentCatalog', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/agentCatalog')>()),
  useAgentCatalog: () => ({ agents: CATALOG, loading: false }),
}));

import { useHarnessOverrides, type HarnessOverrides } from './useHarnessOverrides';

const MACHINE_AGENTS: AgentAvailability[] = [
  { kind: 'opencode', enabled: true, available: true },
  { kind: 'claude-code', enabled: true, available: true },
  { kind: 'hermes', enabled: true, available: true },
  // Installed but switched off, and enabled but not installed: neither may
  // reach the picker, because both fail at spawn rather than at selection.
  { kind: 'codex', enabled: false, available: true },
  { kind: 'antigravity', enabled: true, available: false },
];

const PINS: StepOverride[] = [
  { step_id: 's-implement', agent_kind: 'claude-code', model: 'sonnet', effort: 'max' },
  { step_id: 's-review', agent_kind: 'hermes', model: null, effort: 'high' },
];

function mount(initial: string | null | undefined, pins: StepOverride[] = PINS) {
  return renderHook(
    ({ selectedStepId }: { selectedStepId: string | null | undefined }) =>
      useHarnessOverrides({ selectedStepId, stepOverrides: pins }),
    { initialProps: { selectedStepId: initial } },
  );
}

/** The probe is a once-only latch, so every test that needs `machineAgents` or
 *  a probed model list has to drive it exactly once and wait for it. */
async function probe(result: { current: HarnessOverrides }) {
  act(() => {
    result.current.probeForFeature({ agentKind: 'opencode', projectId: 'p-1' });
  });
  await waitFor(() => expect(result.current.machineAgents.length).toBeGreaterThan(0));
}

beforeEach(() => {
  vi.clearAllMocks();
  getAgentModels.mockResolvedValue([{ value: 'm-1', name: 'm-1' }]);
  getProjectById.mockResolvedValue({ id: 'p-1', remote_host: null });
  listAgentConfigs.mockResolvedValue(MACHINE_AGENTS);
});

describe('useHarnessOverrides', () => {
  it('seeds the controls from the selected node\'s own pin', async () => {
    const { result } = mount('s-implement');

    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));
    expect(result.current.selectedModel).toBe('sonnet');
    expect(result.current.selectedEffort).toBe('max');
  });

  it('shows nothing pinned for a node that has no override', async () => {
    const { result } = mount('s-research');

    await waitFor(() => expect(result.current.selectedAgent).toBe(''));
    expect(result.current.selectedModel).toBe('');
    expect(result.current.selectedEffort).toBe('');
  });

  it('seeds a pin that lands after the selection did', async () => {
    // The ordering `useFeatureRun` actually produces: the steps IPC resolves
    // and the selection lands on this node while `step_overrides` is still the
    // initial `[]`, and the feature row carrying the pins is a second IPC away.
    const { result, rerender } = renderHook(
      ({ pins }: { pins: StepOverride[] }) =>
        useHarnessOverrides({ selectedStepId: 's-implement', stepOverrides: pins }),
      { initialProps: { pins: [] as StepOverride[] } },
    );
    await waitFor(() => expect(result.current.selectedAgent).toBe(''));

    rerender({ pins: PINS });

    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));
    expect(result.current.selectedModel).toBe('sonnet');
    expect(result.current.selectedEffort).toBe('max');
  });

  it('leaves a half-made choice alone when a poll rebuilds pins it did not change', async () => {
    const { result, rerender } = renderHook(
      ({ pins }: { pins: StepOverride[] }) =>
        useHarnessOverrides({ selectedStepId: 's-research', stepOverrides: pins }),
      { initialProps: { pins: PINS } },
    );
    await waitFor(() => expect(result.current.selectedAgent).toBe(''));

    act(() => { result.current.onAgentChange('claude-code'); });
    act(() => { result.current.setSelectedEffort('high'); });
    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));

    // A fresh array every poll, and another node's pin moved — neither is this
    // node's pin, which is still absent before and after.
    rerender({
      pins: [
        { ...PINS[0] },
        { step_id: 's-review', agent_kind: 'opencode', model: null, effort: 'low' },
      ],
    });

    expect(result.current.selectedAgent).toBe('claude-code');
    expect(result.current.selectedEffort).toBe('high');
  });

  it('never carries the previous node\'s choice onto the next node', async () => {
    const { result, rerender } = mount('s-implement');
    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));

    rerender({ selectedStepId: 's-research' });

    await waitFor(() => expect(result.current.selectedAgent).toBe(''));
    expect(result.current.selectedModel).toBe('');
    expect(result.current.selectedEffort).toBe('');
  });

  it('drops an unpinned choice the user made before retargeting', async () => {
    const { result, rerender } = mount('s-research');
    await waitFor(() => expect(result.current.selectedAgent).toBe(''));

    act(() => { result.current.onAgentChange('claude-code'); });
    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));

    rerender({ selectedStepId: 's-plan' });
    await waitFor(() => expect(result.current.selectedAgent).toBe(''));
  });

  it('clamps a seeded effort the pinned harness cannot run', async () => {
    const { result } = mount('s-review');

    // hermes declares an empty ladder, so `reconcileEffort` collapses the
    // pinned level to "inherit" rather than showing one the adapter would drop.
    await waitFor(() => expect(result.current.selectedAgent).toBe('hermes'));
    expect(result.current.selectedEffort).toBe('');
    expect(result.current.retryEffortLevels).toEqual([]);
  });

  it('offers only the harnesses this machine can actually run', async () => {
    const { result } = mount('s-research');
    await probe(result);

    expect(result.current.availableAgents).toEqual(['opencode', 'claude-code', 'hermes']);
  });

  it('re-probes models when the node it retargets to pins another harness', async () => {
    const { result, rerender } = mount('s-research');
    await probe(result);
    expect(getAgentModels).toHaveBeenLastCalledWith('local', 'opencode');

    rerender({ selectedStepId: 's-implement' });

    await waitFor(() => expect(getAgentModels).toHaveBeenLastCalledWith('local', 'claude-code'));
    // The pin survives the re-probe: the list is what went stale, not the choice.
    expect(result.current.selectedModel).toBe('sonnet');
  });

  it('leaves a surface with no per-step selection alone', async () => {
    // The Sync pane mounts its own copy with no selection at all. Seeding it
    // would wipe the resolver harness the user picked on the next render.
    const { result, rerender } = renderHook(() => useHarnessOverrides());

    act(() => { result.current.onAgentChange('claude-code'); });
    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));

    rerender();
    expect(result.current.selectedAgent).toBe('claude-code');
  });

  it('never adopts the feature-wide model into a per-step control', async () => {
    // `useFeatureRun.fetchRun` calls this on every poll with `Feature.model`.
    // On this surface an empty control means *inherit*, so filling it would
    // manufacture a tier-1 pin of a value nobody picked: `useStepAssignment`
    // reads `dirty` off exactly this against `pin?.model ?? ''`, lights Apply,
    // and Apply writes the adopted model as this node's own.
    const { result } = mount('s-research');
    await waitFor(() => expect(result.current.selectedModel).toBe(''));

    act(() => { result.current.adoptFeatureModel('sonnet'); });

    expect(result.current.selectedModel).toBe('');
    expect(result.current.selectedAgent).toBe('');
    expect(result.current.selectedEffort).toBe('');
  });

  it('keeps adopting on a surface with no per-step selection', async () => {
    // The Sync pane's copy has no node to inherit from, so the feature's model
    // is the only default it has; that behaviour predates the per-step pins.
    const { result } = renderHook(() => useHarnessOverrides());

    act(() => { result.current.adoptFeatureModel('sonnet'); });

    expect(result.current.selectedModel).toBe('sonnet');
  });

  it('keeps every member identity-stable across a render that changed none', async () => {
    const { result, rerender } = mount('s-implement');
    await waitFor(() => expect(result.current.selectedAgent).toBe('claude-code'));

    const before = result.current;
    rerender({ selectedStepId: 's-implement' });
    const after = result.current;

    expect(after).toBe(before);
    for (const key of Object.keys(before) as Array<keyof HarnessOverrides>) {
      expect(after[key]).toBe(before[key]);
    }
  });
});
