import { AlertCircle, Cpu, Gauge, Lock, X, Zap } from 'lucide-react';

import { assignmentEffortLabel, assignmentModelLabel } from '../../../lib/runEventAssignments';
import { isAssignable } from '../../../lib/stepAssignment';
import { observedAssignment } from '../../ui/AssignmentChips';
import { HarnessModelPicker } from '../../ui/HarnessModelPicker';
import type { HarnessOverrides } from '../../FeatureDetail/useHarnessOverrides';
import type { StepAssignment } from '../../FeatureDetail/useStepAssignment';
import type { NodeRunStatus } from '../types';

export interface AssignmentSectionProps {
  /**
   * The selected node's step status. Live vs read-only is `isAssignable`'s
   * call (`lib/stepAssignment.ts`), which records why — offering a control the
   * backend would refuse is a button whose only outcome is an error.
   */
  status: string;
  overrides: HarnessOverrides;
  assignment: StepAssignment;
  /**
   * The node's launch evidence — the `agent_spawned` trio `useRunGraph` joined
   * onto its run status. The read-only branch renders this and never the
   * picker: the picker holds what the *user* has typed, which on a running node
   * is a claim about the future at best and an unapplied edit at worst, so
   * printing it under "was spawned with" asserts an execution that did not
   * happen. Absent — no evidence yet — the panel says nothing about the spawn
   * rather than substituting something else for it.
   */
  observed?: Pick<NodeRunStatus, 'agentKind' | 'model' | 'effort'>;
}

const OBSERVED_CHIP =
  'inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-2 py-1 text-[11px] text-slate-300';

/**
 * Re-point one node at a different harness, model or effort while the run is
 * alive.
 *
 * Every choice here is `overrides`' and `assignment`'s; nothing is fetched and
 * nothing is invoked. The harness list is already narrowed to what the
 * feature's machine answered for, and the effort control is already greyed for
 * a harness with no per-invocation ladder (hermes) — both are
 * `useHarnessOverrides`' job, and reproducing either judgement here would give
 * the surface a second opinion about what the machine can run.
 *
 * An untouched control means **inherit**: whatever the rest of the resolution
 * chain gives this node, not a promise that it keeps running on that if the
 * feature is re-pointed later. The harness placeholder names the inherited
 * harness only when `inheritedAgentKind` knows it, and says a bare "Inherit"
 * otherwise — a named guess reads as a fact about this node.
 */
export function AssignmentSection({
  status,
  overrides,
  assignment,
  observed,
}: AssignmentSectionProps) {
  const readOnly = !isAssignable(status);
  // The same question `AssignmentChips` asks before it draws an observed trio,
  // through the same exported rule — so this panel and the chips on the node
  // beside it cannot disagree about whether the node has spawn evidence.
  const spawned = observedAssignment(observed?.agentKind, observed?.effort);
  const spawnedModel = observed?.model;
  const inherited = overrides.inheritedAgentKind.replace(/-/g, ' ');

  return (
    <div className="rounded-xl border border-white/5 bg-black/20 p-3.5">
      <div className="mb-2.5 flex items-center justify-between gap-2">
        <div className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
          Assignment
        </div>
        {readOnly && (
          <span className="inline-flex items-center gap-1 rounded-md border border-white/10 bg-white/5 px-2 py-0.5 text-[10px] font-bold uppercase tracking-wider text-slate-400">
            <Lock className="h-3 w-3" /> Read-only
          </span>
        )}
      </div>

      {readOnly ? (
        <>
          {spawned ? (
            <div className="flex flex-wrap items-center gap-1.5">
              <span title="Harness" className={OBSERVED_CHIP + ' capitalize'}>
                <Cpu className="h-3 w-3 text-slate-500" />
                {spawned.agentKind.replace(/-/g, ' ')}
              </span>
              {spawnedModel !== undefined && (
                <span title="Model" className={OBSERVED_CHIP}>
                  <Zap className="h-3 w-3 text-slate-500" />
                  {assignmentModelLabel(spawnedModel)}
                </span>
              )}
              <span title="Effort" className={OBSERVED_CHIP}>
                <Gauge className="h-3 w-3 text-slate-500" />
                {assignmentEffortLabel(spawned.effort)}
              </span>
            </div>
          ) : (
            <p className="text-[11px] leading-relaxed text-slate-400">
              No launch evidence for this execution yet.
            </p>
          )}
          <p className="mt-2.5 text-[11px] leading-relaxed text-slate-400">
            {spawned
              ? `This node is ${status}: its agent was spawned with the assignment above and cannot be re-pointed mid-flight. Stop it, or wait for it to finish, to reassign it.`
              : `This node is ${status}: it cannot be re-pointed mid-flight. Stop it, or wait for it to finish, to reassign it.`}
          </p>
        </>
      ) : (
        <>
          <HarnessModelPicker
            agentKinds={overrides.availableAgents}
            models={overrides.availableModels}
            modelsLoading={overrides.isLoadingModels}
            agentKind={overrides.selectedAgent}
            model={overrides.selectedModel}
            onAgentKindChange={overrides.onAgentChange}
            onModelChange={overrides.setSelectedModel}
            inheritedAgentKind={overrides.inheritedAgentKind}
            agentPlaceholder={inherited ? `Inherit (${inherited})` : 'Inherit'}
            modelPlaceholder="Inherit (workflow default)"
            effort={overrides.selectedEffort}
            onEffortChange={overrides.setSelectedEffort}
            effortLevels={overrides.retryEffortLevels}
            effortPlaceholder="Inherit"
          />

          <div className="mt-3 flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => void assignment.reset()}
              disabled={assignment.applying || !assignment.pinned}
              className="rounded-lg border border-white/10 bg-white/5 px-3 py-1.5 text-xs font-bold text-slate-300 transition hover:bg-white/10 disabled:cursor-not-allowed disabled:opacity-40"
            >
              Reset to inherited
            </button>
            <button
              type="button"
              onClick={() => void assignment.apply()}
              disabled={assignment.applying || !assignment.dirty}
              className="rounded-lg bg-violet-600 px-3 py-1.5 text-xs font-bold text-white transition hover:bg-violet-500 disabled:cursor-not-allowed disabled:bg-slate-700/40 disabled:text-slate-500"
            >
              {assignment.applying ? 'Applying…' : 'Apply'}
            </button>
          </div>
        </>
      )}

      {assignment.error && (
        <div
          role="alert"
          className="mt-2.5 flex items-start gap-2 rounded-lg border border-rose-500/20 bg-rose-950/10 p-2.5 text-[11px] text-rose-300/90"
        >
          <AlertCircle className="mt-px h-3.5 w-3.5 shrink-0 text-rose-400" />
          <span className="min-w-0 flex-1">{assignment.error}</span>
          <button
            type="button"
            onClick={assignment.clearError}
            aria-label="Dismiss assignment error"
            className="shrink-0 text-rose-400/70 transition hover:text-rose-200"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
    </div>
  );
}

export default AssignmentSection;
