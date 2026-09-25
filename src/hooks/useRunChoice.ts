import { useCallback, useMemo, useRef, useState } from 'react';

import { effortLevelsFor, useAgentCatalog } from '../lib/agentCatalog';
import { getAgentModels } from '../lib/agentModels';
import { reconcileEffort, type EffortLevel } from '../lib/effortLevels';
import { useErrorBus } from '../lib/errorBus';
import type { AgentAvailability } from '../lib/featureDetail';
import type { RunChoice } from '../lib/reviewLaunch';
import type { ConfigOptionValue } from '../types';

export interface RunChoiceInput {
  /** Every harness row this surface's machine answered for. Passed in rather
   *  than fetched: one view holds many launch surfaces, and each one probing
   *  `get_agent_configs` for itself is the same remote round-trip repeated per
   *  row. */
  machineAgents: AgentAvailability[];
  /** The machine the models are probed on — a harness's model list is
   *  per-machine, so the two cannot be resolved apart. */
  machineId: string;
}

export interface RunChoiceState {
  workflowId: string;
  setWorkflowId: React.Dispatch<React.SetStateAction<string>>;
  agentKind: string;
  /** Switching the harness invalidates the model and may invalidate the
   *  effort, so the write goes through here rather than through a plain
   *  setter. */
  setAgentKind: (agentKind: string) => void;
  model: string;
  setModel: React.Dispatch<React.SetStateAction<string>>;
  effort: EffortLevel | '';
  setEffort: React.Dispatch<React.SetStateAction<EffortLevel | ''>>;
  /** The kinds this machine has both switched on and installed. */
  availableAgents: string[];
  availableModels: ConfigOptionValue[];
  isLoadingModels: boolean;
  /** The effort levels the chosen harness accepts. Empty means the harness has
   *  no per-invocation effort control at all and the picker must disable
   *  rather than offer a level the adapter drops. */
  effortLevels: readonly EffortLevel[];
  choice: RunChoice;
}

/**
 * One launch surface's `{ workflowId, agentKind, model, effort }` selection and
 * the per-machine model probe that follows it.
 *
 * **An untouched control is `''`, and `choice` maps `''` to `undefined`.** The
 * difference is load-bearing: {@link RunChoice} reads an unset field as
 * *inherit* — the project default, then the engine's — while `agentKind: ''` is
 * a harness named the empty string, which inherits nothing. Nothing here ever
 * substitutes a concrete default for a control the user did not touch.
 *
 * **Every member of the returned object is identity-stable across a render that
 * changed none of them, and so is the object.** The rows that own one of these
 * are memoized and their list re-renders on every listing update; a fresh
 * object literal per render fails `Object.is` for every prop and re-renders the
 * whole list. `availableAgents` is derived through a string for the same
 * reason — the caller may rebuild `machineAgents` on any render, and the
 * contract above must not depend on it not doing so.
 *
 * Models are probed only once a harness is *chosen*. While the choice is
 * inherit, the harness the run will use is the project's, which this surface
 * has not resolved and must not guess at: a list probed for a guess would be
 * pinned into the run as if the user had picked from it.
 *
 * The sibling of this hook is `components/FeatureDetail/useHarnessOverrides.ts`,
 * which does the same job for a retry or replay and additionally resolves the
 * machine. This one takes the machine as an input instead.
 */
export function useRunChoice(input: RunChoiceInput): RunChoiceState {
  const { machineAgents, machineId } = input;
  const { reportError } = useErrorBus();
  const { agents: agentCatalog } = useAgentCatalog();

  const [workflowId, setWorkflowId] = useState('');
  const [agentKind, setAgentKindState] = useState('');
  const [model, setModel] = useState('');
  const [effort, setEffort] = useState<EffortLevel | ''>('');
  const [availableModels, setAvailableModels] = useState<ConfigOptionValue[]>([]);
  const [isLoadingModels, setIsLoadingModels] = useState(false);

  // Installed *and* enabled. Offering a harness the machine does not have
  // produces a confident description of a run that dies at spawn — the harm
  // that argued this picker out of existence the first time.
  const offered = machineAgents
    .filter((a) => a.enabled && a.available)
    .map((a) => a.kind)
    .join(',');
  const availableAgents = useMemo(() => (offered ? offered.split(',') : []), [offered]);

  const effortLevels = useMemo(
    () => effortLevelsFor(agentCatalog, agentKind),
    [agentCatalog, agentKind],
  );

  // A user flipping the select twice leaves two probes in flight, and the
  // slower one is the first switch. Without the token its list lands last and
  // the surface offers models for a harness the run will not use.
  const probe = useRef(0);

  const setAgentKind = useCallback(
    (kind: string) => {
      setAgentKindState(kind);
      // Models are harness-specific, so neither the pinned one nor the list it
      // came from survives the switch.
      setModel('');
      setAvailableModels([]);
      setEffort((current) => reconcileEffort(current, effortLevelsFor(agentCatalog, kind)));

      const token = ++probe.current;
      if (!kind) return;
      setIsLoadingModels(true);
      (async () => {
        try {
          const models = await getAgentModels(machineId, kind);
          if (probe.current === token) setAvailableModels(models);
        } catch (err) {
          reportError(err, { kind: 'internal' });
        } finally {
          if (probe.current === token) setIsLoadingModels(false);
        }
      })();
    },
    [agentCatalog, machineId, reportError],
  );

  const choice = useMemo<RunChoice>(
    () => ({
      workflowId: workflowId || undefined,
      agentKind: agentKind || undefined,
      model: model || undefined,
      effort: effort || undefined,
    }),
    [workflowId, agentKind, model, effort],
  );

  return useMemo(
    () => ({
      workflowId,
      setWorkflowId,
      agentKind,
      setAgentKind,
      model,
      setModel,
      effort,
      setEffort,
      availableAgents,
      availableModels,
      isLoadingModels,
      effortLevels,
      choice,
    }),
    [
      workflowId,
      agentKind,
      setAgentKind,
      model,
      effort,
      availableAgents,
      availableModels,
      isLoadingModels,
      effortLevels,
      choice,
    ],
  );
}
