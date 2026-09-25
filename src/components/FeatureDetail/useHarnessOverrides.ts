import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { offerableAgentKinds } from '../../lib/agentAvailability';
import { getAgentModels } from '../../lib/agentModels';
import { useErrorBus } from '../../lib/errorBus';
import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { reconcileEffort, type EffortLevel } from '../../lib/effortLevels';
import { getProjectById, listAgentConfigs, type AgentAvailability } from '../../lib/featureDetail';
import type { StepOverride } from '../../types';

export interface HarnessOverrides {
  /** Every harness row the feature's machine answered for, probe result and
   *  containment included. It travels with the selection because the machine
   *  is what this hook resolved and nothing downstream knows it — unlike the
   *  session-wide catalog, which any site fetches for itself through
   *  `useAgentCatalog`. */
  machineAgents: AgentAvailability[];
  availableModels: Array<{ value: string; name: string }>;
  selectedModel: string;
  setSelectedModel: (model: string) => void;
  isLoadingModels: boolean;
  availableAgents: string[];
  selectedAgent: string;
  selectedEffort: EffortLevel | '';
  setSelectedEffort: React.Dispatch<React.SetStateAction<EffortLevel | ''>>;
  /** The effort the last seed wrote — the node's pin, reconciled against the
   *  catalog *as it stood then*. What `dirty` compares against, so a catalog
   *  landing after the seed cannot make an untouched node read as edited. */
  seededEffort: EffortLevel | '';
  featureAgentKind: string;
  /**
   * The harness an untouched harness control resolves to, as far as this hook
   * can know — `''` when it cannot.
   *
   * On the per-step surface that is the feature-wide harness only when the
   * feature actually sets one: tier 2 then outranks everything a node inherits
   * from. Without one, the node's harness is its workflow step's, then the
   * project default's (`driver/resolution.rs`), and neither reaches this hook
   * — so naming `featureAgentKind`'s `'opencode'` fallback here would label
   * the node, and probe its models and pick its effort ladder, for a harness
   * that may not be the one it runs on. The Sync pane's copy keeps
   * `featureAgentKind`, which is what its resolver really falls back to.
   */
  inheritedAgentKind: string;
  retryEffortLevels: readonly EffortLevel[];
  onAgentChange: (agentKind: string) => void;
  adoptFeatureModel: (model: string | null | undefined) => void;
  probeForFeature: (input: { agentKind: string | null | undefined; projectId: string }) => void;
}

export interface HarnessOverridesInput {
  /**
   * The `step_id` the controls address. `null` is a surface whose selection is
   * empty; **absent** is a surface that has no per-step selection at all — the
   * Sync pane's own copy of this hook, which addresses a merge-conflict
   * resolver rather than a node and must never be re-seeded under the user.
   *
   * The two are not interchangeable, exactly as in `useStepSelection`.
   */
  selectedStepId?: string | null;
  /** The run's tier-1 pins (`Feature.step_overrides`), which the selected
   *  node's controls open on. */
  stepOverrides?: StepOverride[];
}

/**
 * The model / harness / effort pinned on **one node at a time** — seeded from
 * that node's existing `StepOverride`, cleared when the selection moves — plus
 * the probe that discovers which of each the feature's machine can offer.
 *
 * The selection is not run-wide. A choice made while reading one node, left
 * standing after the user moved to another, is indistinguishable from a choice
 * made *for* that other node, and the apply path cannot tell them apart either.
 * Called without a `selectedStepId` the hook keeps its older, unaddressed
 * behaviour: a selection that survives until the caller changes it.
 *
 * **Every member of the returned object is identity-stable across a render that
 * changed none of them, and so is the object.** `FeatureDetailView` hands this
 * straight to memoized `StepCard`s, and it re-renders on every step click now
 * that selection routes through `navigate` — a fresh object literal here fails
 * `Object.is` for all of them and re-renders the whole run to move one row's
 * highlight.
 */
export function useHarnessOverrides(input?: HarnessOverridesInput): HarnessOverrides {
  const { reportError } = useErrorBus();
  const [availableModels, setAvailableModels] = useState<Array<{ value: string; name: string }>>([]);
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  // `machineAgents` is what the feature's machine answered for every registered
  // harness; `selectedAgent === ''` means "inherit", i.e. leave the node to the
  // harness the resolution chain already gives it.
  const [machineAgents, setMachineAgents] = useState<AgentAvailability[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>('');
  const [selectedEffort, setSelectedEffort] = useState<EffortLevel | ''>('');
  const { agents: agentCatalog } = useAgentCatalog();
  const [featureAgentKind, setFeatureAgentKind] = useState<string>('opencode');
  const [featureWideAgentKind, setFeatureWideAgentKind] = useState<string>('');
  // `null` until `probeForFeature` resolved the project: probing models on a
  // guessed `'local'` would answer for the wrong machine on a remote project.
  const [featureMachineId, setFeatureMachineId] = useState<string | null>(null);
  // The harness `availableModels` was probed for. Models are harness-specific,
  // so this is what says whether the list still describes the current choice.
  const [probedAgent, setProbedAgent] = useState<string>('');
  const [seededEffort, setSeededEffort] = useState<EffortLevel | ''>('');
  const addressed = input !== undefined && 'selectedStepId' in input;
  const inheritedAgentKind = addressed ? featureWideAgentKind : featureAgentKind;

  const availableAgents = useMemo(() => offerableAgentKinds(machineAgents), [machineAgents]);

  // The effort levels the harness this selection will actually run under
  // accepts. Empty (hermes) disables the control rather than offering a level
  // the adapter would drop on the floor.
  const retryEffortLevels = useMemo(
    () => effortLevelsFor(agentCatalog, selectedAgent || inheritedAgentKind),
    [agentCatalog, selectedAgent, inheritedAgentKind],
  );

  /**
   * The feature's model as the default for an *unaddressed* picker, and
   * nothing at all for a per-step one.
   *
   * `useFeatureRun.fetchRun` calls this on every poll. On the per-step surface
   * `''` means **inherit**, so filling it there manufactures a choice: the
   * control becomes indistinguishable from a real pin, `useStepAssignment`
   * reads `dirty` against `pin?.model ?? ''` and lights Apply, and Apply
   * writes the feature's model as this node's own tier-1 pin — the mirror of
   * the rule that no tier-2 value may be written from this surface. A
   * `reset()` would then bounce back on the next poll.
   */
  const adopt = useRef({ addressed: false, selectedModel });
  adopt.current = { addressed, selectedModel };
  const adoptFeatureModel = useCallback((model: string | null | undefined) => {
    if (adopt.current.addressed || adopt.current.selectedModel !== '') return;
    setSelectedModel(model || '');
  }, []);

  // The harness a request was last made for. An answer for any other one is
  // for a choice the user has since left, and landing it would put that
  // harness's models under this one's name.
  const requestedAgent = useRef('');
  const refreshModels = useCallback((agentKind: string, machineId: string) => {
    requestedAgent.current = agentKind;
    setIsLoadingModels(true);
    (async () => {
      try {
        const models = await getAgentModels(machineId, agentKind);
        if (requestedAgent.current !== agentKind) return;
        setAvailableModels(models as Array<{ value: string; name: string }>);
        setProbedAgent(agentKind);
      } catch (err) {
        reportError(err, { kind: "internal" });
      } finally {
        if (requestedAgent.current === agentKind) setIsLoadingModels(false);
      }
    })();
  }, [reportError]);

  // `useFeatureRun.fetchRun` calls this on every poll; only the first that
  // succeeds does anything. A failure re-opens it for the next poll.
  const featureProbe = useRef<'idle' | 'inflight' | 'done'>('idle');
  const probeForFeature = useCallback((feature: { agentKind: string | null | undefined; projectId: string }) => {
    if (featureProbe.current !== 'idle') return;
    featureProbe.current = 'inflight';
    setFeatureAgentKind(feature.agentKind || 'opencode');
    setFeatureWideAgentKind(feature.agentKind || '');
    (async () => {
      try {
        const project = await getProjectById(feature.projectId);
        const machineId = project?.remote_host || 'local';
        // A missing agent-config list is non-fatal — we just won't show the
        // harness picker.
        const configs = await listAgentConfigs({ machineId, refresh: false }).catch(() => []);
        setMachineAgents(configs || []);
        setFeatureMachineId(machineId);
        featureProbe.current = 'done';
      } catch (err) {
        featureProbe.current = 'idle';
        reportError(err, { kind: "internal" });
      }
    })();
  }, [reportError]);

  /**
   * The model list follows the harness the control resolves to — whatever
   * moved it: the feature probe landing, a pick, a retarget, or a pin that
   * arrived after the selection. One rule rather than a re-probe at each of
   * those sites, because the sites race: `fetchRun` commits the selection and
   * then the pins before `probeForFeature` resolves the machine, so a re-probe
   * gated on "a list was already probed" is skipped on exactly the first
   * load, and the pinned node lists the feature harness's models.
   */
  const modelsFor = selectedAgent || inheritedAgentKind;
  useEffect(() => {
    if (featureMachineId === null) return;
    if (modelsFor === '' || modelsFor === probedAgent) {
      // Nothing to fetch, so disown whatever is still in flight: it answers
      // for a harness the control has since left.
      requestedAgent.current = modelsFor;
      setIsLoadingModels(false);
      return;
    }
    if (modelsFor === requestedAgent.current) return;
    refreshModels(modelsFor, featureMachineId);
  }, [modelsFor, featureMachineId, probedAgent, refreshModels]);

  // Switching the harness invalidates the model selection (models are
  // harness-specific); the effect above re-probes the list.
  const onAgentChange = useCallback((agentKind: string) => {
    setSelectedAgent(agentKind);
    setSelectedModel('');
    // Clamp the effort to what the chosen harness actually accepts, so a level
    // the previous harness supported doesn't linger in a now-greyed or
    // mismatched control and get silently re-sent.
    setSelectedEffort((e) => reconcileEffort(e, effortLevelsFor(agentCatalog, agentKind || inheritedAgentKind)));
  }, [agentCatalog, inheritedAgentKind]);

  const seed = useRef({ agentCatalog, inheritedAgentKind });
  seed.current = { agentCatalog, inheritedAgentKind };

  const selectedStepId = input?.selectedStepId;
  const pin =
    selectedStepId === undefined || selectedStepId === null
      ? undefined
      : input?.stepOverrides?.find((o) => o.step_id === selectedStepId);
  const pinnedAgent = pin?.agent_kind ?? null;
  const pinnedModel = pin?.model ?? null;
  const pinnedEffort = pin?.effort ?? null;

  /**
   * Retarget — and a change to *this* node's pin — move the controls.
   *
   * The catalog and the inherited harness land asynchronously under
   * a selection the user is still making, so the seed reads them through a
   * ref: an effect depending on them would overwrite a half-made choice each
   * time one arrived. `step_overrides` cannot be a ref read on the same
   * reasoning, because the array the first render sees is `[]` —
   * `useFeatureRun.fetchRun` commits the steps, and with them the selection,
   * one IPC before the feature row that carries the pins. A seed that never
   * re-runs leaves a pinned node reading "Inherit" for the life of the
   * selection, with Apply lit against the loaded pin and armed to submit the
   * all-`null` trio that *deletes* it.
   *
   * So the dependency is the pin's three values and never the array: a poll
   * rebuilds the array, not them.
   *
   * The pin is seeded whatever the probe has said so far — it is the run's own
   * record of a harness that already ran, and the availability answer lands
   * after this, so gating on it would silently drop a real pin for the time it
   * takes to arrive.
   */
  useEffect(() => {
    if (selectedStepId === undefined) return;
    const { agentCatalog: catalog, inheritedAgentKind: inherited } = seed.current;
    const agentKind = pinnedAgent ?? '';
    const effort = reconcileEffort(pinnedEffort ?? '', effortLevelsFor(catalog, agentKind || inherited));
    setSelectedAgent(agentKind);
    setSelectedModel(pinnedModel ?? '');
    setSelectedEffort(effort);
    setSeededEffort(effort);
  }, [selectedStepId, pinnedAgent, pinnedModel, pinnedEffort]);

  return useMemo(
    () => ({
      machineAgents,
      availableModels,
      selectedModel,
      setSelectedModel,
      isLoadingModels,
      availableAgents,
      selectedAgent,
      selectedEffort,
      setSelectedEffort,
      seededEffort,
      featureAgentKind,
      inheritedAgentKind,
      retryEffortLevels,
      onAgentChange,
      adoptFeatureModel,
      probeForFeature,
    }),
    [
      machineAgents,
      availableModels,
      isLoadingModels,
      availableAgents,
      selectedAgent,
      selectedEffort,
      selectedModel,
      seededEffort,
      featureAgentKind,
      inheritedAgentKind,
      retryEffortLevels,
      onAgentChange,
      adoptFeatureModel,
      probeForFeature,
    ],
  );
}
