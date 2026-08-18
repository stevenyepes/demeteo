import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import type { HarnessOverrides } from './useHarnessOverrides';

/**
 * The harness, model and effort one conflict resolution runs under, offered
 * beside the "Resolve with agent" button.
 *
 * `HarnessModelPicker` is the control set; what only this call site knows is
 * what an untouched control means here. Blank is not "inherit a default" and
 * not "keep what the feature runs with" either: the resolver is a role, so a
 * blank row falls through the project's own conflict-resolver setting first —
 * which is why the placeholders name inheritance rather than a harness.
 *
 * Which harness is picked also decides how tightly the turn is confined to the
 * sync worktree: opencode and hermes are denied every directory outside it and
 * codex is sandboxed to it, while claude-code and pi are given no path fence at
 * all. `adapters/step_executor/sync_resolve.rs` records that at the spawn.
 */
export function SyncResolverOptions({ overrides }: { overrides: HarnessOverrides }) {
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
      agentPlaceholder="Inherit (project, then run)"
      modelPlaceholder="Inherit"
      effort={overrides.selectedEffort}
      onEffortChange={overrides.setSelectedEffort}
      effortLevels={overrides.retryEffortLevels}
      effortPlaceholder="Inherit"
    />
  );
}

export default SyncResolverOptions;
