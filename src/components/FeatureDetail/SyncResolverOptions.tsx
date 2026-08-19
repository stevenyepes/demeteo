import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import type { SyncResolverSelection } from './useSyncResolverOverrides';

/**
 * The harness, model and effort one conflict resolution runs under, offered
 * above the "Resolve with agent" row in the Sync pane.
 *
 * `HarnessModelPicker` is the control set; what only this call site knows is
 * what an untouched control means here. Blank is not "inherit a default" and
 * not "keep what the feature runs with" either: the resolver is a role, so a
 * blank row falls through the project's own conflict-resolver setting first.
 * That is why the inherited harness is named from `selection.inherited` — the
 * backend's own answer — rather than from the feature: the two differ exactly
 * when the setting is set, and it is the label, the model list and the effort
 * ladder that would all be wrong.
 *
 * Which harness is picked also decides how tightly the turn is confined to the
 * sync worktree: opencode and hermes are denied every directory outside it and
 * codex is sandboxed to it, while claude-code and pi are given no path fence at
 * all. `adapters/step_executor/sync_resolve.rs` records that at the spawn.
 */
export function SyncResolverOptions({ selection }: { selection: SyncResolverSelection }) {
  const { overrides, inherited } = selection;
  const inheritedKind = inherited?.agent_kind ?? '';
  return (
    <HarnessModelPicker
      agentKinds={overrides.availableAgents}
      models={overrides.availableModels}
      modelsLoading={overrides.isLoadingModels}
      agentKind={overrides.selectedAgent}
      model={overrides.selectedModel}
      onAgentKindChange={overrides.onAgentChange}
      onModelChange={overrides.setSelectedModel}
      inheritedAgentKind={inheritedKind}
      agentPlaceholder={inheritedKind ? `Inherit (${inheritedKind.replace(/-/g, ' ')})` : 'Inherit'}
      modelPlaceholder={inherited?.model ? `Inherit (${inherited.model})` : 'Inherit'}
      effort={overrides.selectedEffort}
      onEffortChange={overrides.setSelectedEffort}
      effortLevels={overrides.retryEffortLevels}
      effortPlaceholder={inherited ? `Inherit (${inherited.effort})` : 'Inherit'}
    />
  );
}

export default SyncResolverOptions;
