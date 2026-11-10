import { useState } from 'react';

import { Button, buttonClasses } from '../../../src/components/ui/Button';
import { EFFORT_LABELS } from '../../../src/lib/effortLevels';
import { GITLAB_RUNNER_PROBLEM, usdOfCents } from '../lib/dispatchForm';
import type { HubProject } from '../lib/projects';
import { routeHref } from '../lib/router';
import type { HubWorkflow } from '../lib/workflows';

interface ProjectRowProps {
  project: HubProject;
  workflows: HubWorkflow[];
  disabled: boolean;
  onRemove: (id: string) => void;
}

/** Removing asks twice, as a Git host does; a run already sent from the project is not stopped by it. */
export function ProjectRow({ project, workflows, disabled, onRemove }: ProjectRowProps) {
  const [confirming, setConfirming] = useState(false);
  const defaults = defaultsOf(project, workflows);
  return (
    <li className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <p className="text-sm text-slate-100 break-words">{project.name}</p>
        <p className="font-mono text-xs text-slate-400 break-all">
          {project.host}/{project.fullName}
        </p>
        <p className="text-xs text-slate-400">{defaults.length === 0 ? 'No defaults' : defaults.join(' · ')}</p>
        {!project.runnerReady && <p className="text-xs text-slate-300">{GITLAB_RUNNER_PROBLEM}</p>}
      </div>
      {confirming ? (
        <>
          <Button variant="danger" size="sm" disabled={disabled} onClick={() => onRemove(project.id)}>
            Remove from this Hub
          </Button>
          <Button variant="ghost" size="sm" disabled={disabled} onClick={() => setConfirming(false)}>
            Keep
          </Button>
        </>
      ) : (
        <>
          <a href={routeHref({ name: 'project', projectId: project.id })} className={buttonClasses('secondary', 'sm')}>
            Edit
          </a>
          <Button variant="danger-subtle" size="sm" disabled={disabled} onClick={() => setConfirming(true)}>
            Remove
          </Button>
        </>
      )}
    </li>
  );
}

/** A workflow id no starter carries any more reads as unset, as the Hub reads it. */
function defaultsOf(project: HubProject, workflows: HubWorkflow[]): string[] {
  const workflow = workflows.find((choice) => choice.workflowId === project.defaultWorkflowId);
  const cost = usdOfCents(project.defaultMaxBudgetCents ?? 0);
  return [
    workflow?.name,
    project.defaultAgentKind,
    project.defaultModel,
    project.defaultEffort === null ? null : `${EFFORT_LABELS[project.defaultEffort]} effort`,
    cost === '' ? null : `$${cost} max`,
  ].filter((part): part is string => typeof part === 'string' && part !== '');
}
