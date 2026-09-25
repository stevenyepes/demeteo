/**
 * The load ordering that ships, which no other suite reproduces.
 *
 * `useFeatureRun.fetchRun` commits a render after `setSteps(...)` — and with it
 * the selection — one IPC before `getFeature` answers with the pins. Every
 * other frontend suite hands `stepOverrides` to `renderHook` or to `render` at
 * mount, i.e. in the one sequence production never produces, which is why a
 * pinned node reading "Inherit" with Apply armed to submit the all-`null` trio
 * that deletes the pin survived a green gate.
 *
 * Both hooks and the section are the real ones. `dirty`, Apply's disabled state
 * and Reset's are each derived somewhere else from the same seed, and a test
 * that recomputed any of them would agree with itself rather than with the
 * screen.
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AgentCatalogEntry } from '../../lib/agentCatalog';
import type { AgentAvailability } from '../../lib/featureDetail';
import type { Project, StepOverride } from '../../types';

const getAgentModels = vi.fn<(machineId: string, agentKind: string) => Promise<unknown>>();
const getProjectById = vi.fn<(projectId: string) => Promise<Partial<Project> | null>>();
const listAgentConfigs = vi.fn<(input: unknown) => Promise<AgentAvailability[]>>();
const setStepAssignment = vi.fn<(input: unknown) => Promise<void>>();
const remoteSetStepAssignment = vi.fn<(input: unknown) => Promise<void>>();

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

vi.mock('../../lib/features', () => ({
  setStepAssignment: (input: unknown) => setStepAssignment(input),
  remoteSetStepAssignment: (input: unknown) => remoteSetStepAssignment(input),
}));

import { AssignmentSection } from '../canvas/nodePanel/AssignmentSection';
import { useHarnessOverrides, type HarnessOverrides } from './useHarnessOverrides';
import { useStepAssignment } from './useStepAssignment';

const PINS: StepOverride[] = [
  { step_id: 's-implement', agent_kind: 'claude-code', model: 'sonnet', effort: 'max' },
];

function Inspector({
  stepOverrides,
  exposed,
}: {
  stepOverrides: StepOverride[];
  /** Where `fetchRun`'s stand-in reaches the probe, as `overridesRef` does. */
  exposed?: { current: HarnessOverrides | null };
}) {
  const overrides = useHarnessOverrides({ selectedStepId: 's-implement', stepOverrides });
  if (exposed) exposed.current = overrides;
  const assignment = useStepAssignment({
    overrides,
    selectedStepId: 's-implement',
    selectedExecutionId: 'se-1',
    stepOverrides,
    remoteRun: null,
    reload: () => {},
  });
  return <AssignmentSection status="pending" overrides={overrides} assignment={assignment} />;
}

beforeEach(() => {
  vi.clearAllMocks();
  // Disjoint per harness, so a list probed for the wrong one cannot pass.
  getAgentModels.mockImplementation(async (_machineId, agentKind) =>
    agentKind === 'claude-code'
      ? [{ value: 'sonnet', name: 'sonnet' }]
      : [{ value: 'gpt-oss', name: 'gpt-oss' }],
  );
  getProjectById.mockResolvedValue({ id: 'p-1', remote_host: null });
  listAgentConfigs.mockResolvedValue([
    { kind: 'opencode', enabled: true, available: true },
    { kind: 'claude-code', enabled: true, available: true },
  ]);
});

describe('the Assignment panel on first load', () => {
  it('opens on the pin for a node selected before the feature row landed', async () => {
    const { rerender } = render(<Inspector stepOverrides={[]} />);

    rerender(<Inspector stepOverrides={PINS} />);

    await waitFor(() =>
      expect(screen.getByLabelText('Harness')).toHaveValue('claude-code'),
    );
    expect(screen.getByLabelText('Model')).toHaveValue('sonnet');
    expect(screen.getByLabelText('Effort')).toHaveValue('max');
    // Nothing was touched, so there is nothing to write — and the node is
    // pinned, so there is something to clear. The pair is the whole defect:
    // reversed, Apply submits the reset and Reset is the one greyed out.
    expect(screen.getByRole('button', { name: 'Apply' })).toBeDisabled();
    expect(screen.getByRole('button', { name: /reset to inherited/i })).toBeEnabled();
  });

  /**
   * `fetchRun` in full: steps (and with them the selection), then the pins,
   * then `probeForFeature` last — which probes the *feature's* harness. The
   * node is pinned to another one, so the list has to follow the pin once the
   * probe lands, and nothing about the selection moves to prompt it.
   */
  async function loadRun(featureAgentKind: string | null) {
    const exposed: { current: HarnessOverrides | null } = { current: null };
    const { rerender } = render(<Inspector stepOverrides={[]} exposed={exposed} />);
    rerender(<Inspector stepOverrides={PINS} exposed={exposed} />);
    await waitFor(() => expect(screen.getByLabelText('Harness')).toHaveValue('claude-code'));
    act(() => {
      exposed.current?.probeForFeature({ agentKind: featureAgentKind, projectId: 'p-1' });
    });
  }

  const modelOptions = () =>
    Array.from((screen.getByLabelText('Model') as HTMLSelectElement).options).map((o) => o.value);

  it('lists the pinned harness\'s models, not the feature\'s, once the probe lands', async () => {
    await loadRun('opencode');

    await waitFor(() => expect(getAgentModels).toHaveBeenLastCalledWith('local', 'claude-code'));
    await waitFor(() => expect(modelOptions()).toContain('sonnet'));
    expect(modelOptions()).not.toContain('gpt-oss');
    expect(screen.getByLabelText('Model')).toHaveValue('sonnet');
  });

  it('names no inherited harness when the feature sets none', async () => {
    await loadRun(null);
    await waitFor(() => expect(getAgentModels).toHaveBeenCalled());

    const harness = screen.getByLabelText('Harness') as HTMLSelectElement;
    expect(harness.options[0]).toHaveTextContent(/^Inherit$/);
  });
});

