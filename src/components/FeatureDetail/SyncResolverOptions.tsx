import { HarnessContainmentNote } from './HarnessContainmentNote';
import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import { useAgentCatalog } from '../../lib/agentCatalog';
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
 * sync worktree, which is why the choice carries a containment note: each
 * harness declares its own answer as a `PathContainment`, so no surface keeps a
 * list of who is fenced. The note reads that answer off the rows the *feature's
 * machine* returned rather than off the catalog (`lib/pathContainment.ts`).
 */
export function SyncResolverOptions({ selection }: { selection: SyncResolverSelection }) {
  const { overrides, inherited } = selection;
  const { agents } = useAgentCatalog();
  const inheritedKind = inherited?.agent_kind ?? '';
  return (
    <div className="space-y-2">
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
      <HarnessContainmentNote
        agents={agents}
        machineAgents={overrides.machineAgents}
        kind={overrides.selectedAgent || inheritedKind}
      />
    </div>
  );
}

export default SyncResolverOptions;
