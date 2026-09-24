import { Workflow as WorkflowIcon } from 'lucide-react';

import { FieldLabel } from '../ui/FieldLabel';
import { HarnessModelPicker } from '../ui/HarnessModelPicker';
import type { RunChoiceInput, RunChoiceState } from '../../hooks/useRunChoice';
import type { WorkflowWithSteps } from '../../types';

/**
 * What one review view fetched once, for every launch surface inside it to
 * read — the workflows and the machine whose probe answered for the harnesses.
 *
 * One object rather than three props because the rows that carry it are
 * `memo`ized: three props are three references the parent has to keep stable,
 * and the one that gets forgotten re-renders the whole queue on every listing
 * update. `machineAgents` and `machineId` are {@link RunChoiceInput}'s, and are
 * documented there.
 */
export interface ReviewRunInputs extends RunChoiceInput {
  /** The whole list, as `listWorkflows` returned it. Each surface narrows it
   *  before offering it — a review one through `reviewWorkflowChoices`, which
   *  records why. */
  workflows: WorkflowWithSteps[];
}

export interface ReviewRunOptionsProps {
  /** The selection and the probe behind it, from `useRunChoice`. Nothing is
   *  chosen here; this renders what that hook holds and reports back to it. */
  choice: RunChoiceState;
  /** Already narrowed by the owner — `reviewWorkflowChoices` for a review
   *  surface — so this offers the list it is handed, whole. */
  workflows: WorkflowWithSteps[];
}

/**
 * The workflow, harness, model and effort a review or fix run is launched
 * with, offered beside the button that launches it.
 *
 * `HarnessModelPicker` is the control set for the last three — probe state, and
 * the greyed effort control for a harness with no per-invocation one, are
 * already its job. What only this call site knows is what an *untouched*
 * control means here: **inherit the project default**, and not `RerunOptions`'
 * "keep what the feature is already running with". The difference is that there
 * is no concrete harness to name: the default this surface would inherit lives
 * in project settings it has not read, and naming a guess would pin a run shape
 * the user never chose. So the workflow, harness and effort placeholders say
 * *default* without saying which; a blank one would read as "none", which is
 * the one thing `''` does not mean.
 *
 * The model is the exception. It is only selectable once a harness is chosen,
 * and a chosen harness inherits no model: the project's default belongs to the
 * project's harness, and the engine would hand it to this one regardless. So
 * its placeholder asks for a choice rather than offering a default, and the
 * owning surface holds the launch until one is made — `runChoiceGap`.
 *
 * Shared by both review surfaces rather than spelled twice, which is also what
 * keeps either of them from reaching the ~400 LOC extraction rule.
 */
export function ReviewRunOptions({ choice, workflows }: ReviewRunOptionsProps) {
  return (
    <div className="space-y-3">
      <div>
        <FieldLabel icon={<WorkflowIcon className="w-3 h-3" />}>Workflow</FieldLabel>
        <select
          aria-label="Workflow"
          value={choice.workflowId}
          onChange={(e) => choice.setWorkflowId(e.target.value)}
          className="w-full rounded-lg border border-white/10 bg-black/40 px-3 py-2 text-sm text-white focus:border-cyan-500/50 focus:outline-none"
        >
          <option value="">Default workflow</option>
          {workflows.map((workflow) => (
            <option key={workflow.id} value={workflow.id}>
              {workflow.name}
            </option>
          ))}
        </select>
      </div>

      <HarnessModelPicker
        agentKinds={choice.availableAgents}
        models={choice.availableModels}
        modelsLoading={choice.isLoadingModels}
        agentKind={choice.agentKind}
        model={choice.model}
        onAgentKindChange={choice.setAgentKind}
        onModelChange={choice.setModel}
        agentPlaceholder="Project default"
        modelPlaceholder="Choose a model"
        effort={choice.effort}
        onEffortChange={choice.setEffort}
        effortLevels={choice.effortLevels}
        effortPlaceholder="Default effort"
      />
    </div>
  );
}

export default ReviewRunOptions;
