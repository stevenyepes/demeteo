import { useCallback, useMemo, useState } from 'react';
import { getAgentModels } from '../../lib/agentModels';
import { useErrorBus } from '../../lib/errorBus';
import { effortLevelsFor, useAgentCatalog } from '../../lib/agentCatalog';
import { reconcileEffort, type EffortLevel } from '../../lib/effortLevels';
import { getProjectById, listAgentConfigs, type AgentAvailability } from '../../lib/featureDetail';

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
  featureAgentKind: string;
  retryEffortLevels: readonly EffortLevel[];
  onAgentChange: (agentKind: string) => void;
  adoptFeatureModel: (model: string | null | undefined) => void;
  probeForFeature: (input: { agentKind: string | null | undefined; projectId: string }) => void;
}

/**
 * The model / harness / effort a retry or replay will re-pin, and the probe
 * that discovers which of each the feature's machine can actually offer.
 *
 * **Every member of the returned object is identity-stable across a render that
 * changed none of them, and so is the object.** `FeatureDetailView` hands this
 * straight to memoized `StepCard`s, and it re-renders on every step click now
 * that selection routes through `navigate` — a fresh object literal here fails
 * `Object.is` for all of them and re-renders the whole run to move one row's
 * highlight.
 */
export function useHarnessOverrides(): HarnessOverrides {
  const { reportError } = useErrorBus();
  const [availableModels, setAvailableModels] = useState<Array<{ value: string; name: string }>>([]);
  const [selectedModel, setSelectedModel] = useState<string>('');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  // Harness (coding agent) selection for replay/retry. `machineAgents` is what
  // the feature's machine answered for every registered harness;
  // `selectedAgent === ''` means "keep the feature's current harness".
  // `featureAgentKind` / `featureMachineId` are captured so a harness switch
  // can re-probe models.
  const [machineAgents, setMachineAgents] = useState<AgentAvailability[]>([]);
  const [selectedAgent, setSelectedAgent] = useState<string>('');
  // Re-pin the feature-wide effort on a retry/replay, exactly as the model and
  // harness selects do. `''` keeps whatever the feature already carries.
  const [selectedEffort, setSelectedEffort] = useState<EffortLevel | ''>('');
  const { agents: agentCatalog } = useAgentCatalog();
  const [featureAgentKind, setFeatureAgentKind] = useState<string>('opencode');
  const [featureMachineId, setFeatureMachineId] = useState<string>('local');

  // Installed *and* enabled: a picker offering a harness the machine does not
  // have fails at spawn instead.
  const availableAgents = useMemo(
    () => machineAgents.filter((a) => a.enabled && a.available).map((a) => a.kind),
    [machineAgents],
  );

  // The effort levels the harness the rerun will actually use accepts. Empty
  // (hermes) disables the control rather than offering a level the adapter
  // would drop on the floor.
  const retryEffortLevels = useMemo(
    () => effortLevelsFor(agentCatalog, selectedAgent || featureAgentKind),
    [agentCatalog, selectedAgent, featureAgentKind],
  );

  const adoptFeatureModel = useCallback((model: string | null | undefined) => {
    if (selectedModel === '') {
      setSelectedModel(model || '');
    }
  }, [selectedModel]);

  const probeForFeature = useCallback((input: { agentKind: string | null | undefined; projectId: string }) => {
    if (availableModels.length > 0 || isLoadingModels) return;
    setIsLoadingModels(true);
    const agentKind = input.agentKind || 'opencode';
    setFeatureAgentKind(agentKind);
    (async () => {
      try {
        const project = await getProjectById(input.projectId);
        const machineId = project?.remote_host || 'local';
        setFeatureMachineId(machineId);
        // Probe models for the current harness and, in parallel, fetch which
        // harnesses are actually available on this machine so replay/retry
        // only offer ones that will run. A missing agent-config list is
        // non-fatal — we just won't show the harness picker.
        const [models, configs] = await Promise.all([
          getAgentModels(machineId, agentKind),
          listAgentConfigs({ machineId, refresh: false }).catch(() => []),
        ]);
        setAvailableModels(models as Array<{ value: string; name: string }>);
        setMachineAgents(configs || []);
      } catch (err) {
        reportError(err, { kind: "internal" });
      } finally {
        setIsLoadingModels(false);
      }
    })();
  }, [availableModels.length, isLoadingModels, reportError]);

  // Switching the harness invalidates the probed model list (models are
  // harness-specific), so clear the model selection and re-probe for the
  // chosen harness. An empty choice falls back to the feature's current harness.
  const onAgentChange = useCallback((agentKind: string) => {
    setSelectedAgent(agentKind);
    setSelectedModel('');
    // Clamp the re-pinned effort to what the rerun's harness actually accepts,
    // so a level the previous harness supported doesn't linger in a now-greyed
    // or mismatched control and get silently re-sent.
    setSelectedEffort((e) => reconcileEffort(e, effortLevelsFor(agentCatalog, agentKind || featureAgentKind)));
    setIsLoadingModels(true);
    (async () => {
      try {
        const models = await getAgentModels(featureMachineId, agentKind || featureAgentKind);
        setAvailableModels(models as Array<{ value: string; name: string }>);
      } catch (err) {
        reportError(err, { kind: "internal" });
      } finally {
        setIsLoadingModels(false);
      }
    })();
  }, [agentCatalog, featureAgentKind, featureMachineId, reportError]);

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
      featureAgentKind,
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
      featureAgentKind,
      retryEffortLevels,
      onAgentChange,
      adoptFeatureModel,
      probeForFeature,
    ],
  );
}
