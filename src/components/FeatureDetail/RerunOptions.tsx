import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import type { HarnessOverrides } from './useHarnessOverrides';

/**
 * The harness, model and effort a retry re-pins, offered beside the retry
 * itself.
 *
 * `HarnessModelPicker` is the control set — probe state, and the greyed effort
 * control for a harness with no per-invocation one, are already its job. What
 * only this call site knows is what an *untouched* control means here: not
 * "inherit a default" as everywhere else the picker is used, but "keep what the
 * feature is already running with", which is a concrete harness the user can be
 * shown by name. Hence the placeholders; a blank one would read as "none".
 */
export function RerunOptions({ overrides }: { overrides: HarnessOverrides }) {
  return (
    <HarnessModelPicker
      agentKinds={overrides.availableAgents}
      models={overrides.availableModels}
      modelsLoading={overrides.isLoadingModels}
      agentKind={overrides.selectedAgent}
      model={overrides.selectedModel}
      onAgentKindChange={overrides.onAgentChange}
      onModelChange={overrides.setSelectedModel}
      inheritedAgentKind={overrides.featureAgentKind}
      agentPlaceholder={`Default (${overrides.featureAgentKind.replace(/-/g, ' ')})`}
      modelPlaceholder="Default (from workflow)"
      effort={overrides.selectedEffort}
      onEffortChange={overrides.setSelectedEffort}
      effortLevels={overrides.retryEffortLevels}
      effortPlaceholder="Keep current effort"
    />
  );
}

export default RerunOptions;
