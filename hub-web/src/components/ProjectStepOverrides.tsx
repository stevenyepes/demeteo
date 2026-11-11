import { useId, useState } from 'react';

import {
  EFFORT_LABELS,
  EFFORT_LEVELS,
  type EffortLevel,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import { agentKindsWith, overridableSteps } from '../lib/projectForm';
import type { ProjectStepOverride } from '../lib/projects';
import type { HubWorkflow } from '../lib/workflows';

interface ProjectStepOverridesProps {
  workflows: HubWorkflow[];
  overrides: ProjectStepOverride[];
  /** The Workflow shown first: the project's default when it has one. */
  defaultWorkflowId: string;
  /** What a step whose agent inherits runs on; blank when the project has no default. */
  defaultAgentKind: string;
  onChange: (next: ProjectStepOverride[]) => void;
}

const SELECT_CLASS = 'input-field disabled:cursor-not-allowed disabled:opacity-40';

/**
 * The desktop's Overrides tab, for the Hub's own Workflows. A dispatch that
 * overrides a step itself replaces the project's entry for that step whole,
 * and an entry left with nothing set is removed rather than kept empty.
 */
export function ProjectStepOverrides({
  workflows,
  overrides,
  defaultWorkflowId,
  defaultAgentKind,
  onChange,
}: ProjectStepOverridesProps) {
  const heading = useId();
  const pickerId = useId();
  const withSteps = workflows.filter((workflow) => overridableSteps(workflow).length > 0);
  const [shownId, setShownId] = useState(
    () => withSteps.find((workflow) => workflow.workflowId === defaultWorkflowId)?.workflowId ?? withSteps[0]?.workflowId ?? '',
  );
  const shown = withSteps.find((workflow) => workflow.workflowId === shownId);
  const countFor = (workflowId: string) => overrides.filter((entry) => entry.workflowId === workflowId).length;

  const setStep = (stepId: string, next: Omit<ProjectStepOverride, 'workflowId' | 'stepId'>) => {
    const others = overrides.filter((entry) => !(entry.workflowId === shownId && entry.stepId === stepId));
    const empty = next.agentKind === null && next.model === null && next.effort === null;
    onChange(empty ? others : [...others, { workflowId: shownId, stepId, ...next }]);
  };

  return (
    <section aria-labelledby={heading} className="panel flex min-w-0 flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 id={heading} className="font-heading text-base font-semibold text-white">
          Step overrides
        </h2>
        <p className="text-xs text-slate-400">
          The agent, model and effort a step runs on, in place of the run defaults. A dispatch can still override a
          step itself.
        </p>
      </div>
      {withSteps.length === 0 ? (
        <p className="text-xs text-slate-400">No Workflow this Hub offers has a step that runs an agent.</p>
      ) : (
        <>
          <div className="flex min-w-0 flex-col gap-1.5">
            <label htmlFor={pickerId} className="text-xs font-medium text-slate-300">
              Workflow
            </label>
            <select
              id={pickerId}
              value={shownId}
              onChange={(event) => setShownId(event.target.value)}
              className="input-field"
            >
              {withSteps.map((workflow) => {
                const count = countFor(workflow.workflowId);
                return (
                  <option key={workflow.workflowId} value={workflow.workflowId}>
                    {count === 0 ? workflow.name : `${workflow.name} (${count} overridden)`}
                  </option>
                );
              })}
            </select>
          </div>
          <ul className="flex flex-col gap-3">
            {overridableSteps(shown).map((step) => (
              <StepRow
                key={`${shownId}/${step.id}`}
                title={step.title}
                stepId={step.id}
                entry={overrides.find((entry) => entry.workflowId === shownId && entry.stepId === step.id)}
                defaultAgentKind={defaultAgentKind}
                onChange={(next) => setStep(step.id, next)}
              />
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

function StepRow({
  title,
  stepId,
  entry,
  defaultAgentKind,
  onChange,
}: {
  title: string;
  stepId: string;
  entry: ProjectStepOverride | undefined;
  defaultAgentKind: string;
  onChange: (next: Omit<ProjectStepOverride, 'workflowId' | 'stepId'>) => void;
}) {
  const ids = { agent: useId(), model: useId(), effort: useId() };
  const agentKind = entry?.agentKind ?? '';
  const model = entry?.model ?? '';
  const effort = entry?.effort ?? '';
  const effortsFor = (agent: string) => (agent === '' ? EFFORT_LEVELS : supportedEffortsFor(agent));
  const efforts = effortsFor(agentKind || defaultAgentKind);
  const current = { agentKind: entry?.agentKind ?? null, model: entry?.model ?? null, effort: entry?.effort ?? null };

  const changeAgent = (next: string) => {
    const kept = reconcileEffort(effort, effortsFor(next || defaultAgentKind));
    // A model names one harness's model, so it does not survive a change of harness.
    onChange({ agentKind: next || null, model: next === agentKind ? current.model : null, effort: kept || null });
  };

  return (
    <li className="nested-card flex flex-col gap-3 p-4">
      <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
        <span className="text-sm text-slate-100">{title}</span>
        <span className="font-mono text-xs text-slate-500">{stepId}</span>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <div className="flex min-w-0 flex-col gap-1.5">
          <label htmlFor={ids.agent} className="text-xs font-medium text-slate-300">
            Agent
          </label>
          <select
            id={ids.agent}
            value={agentKind}
            onChange={(event) => changeAgent(event.target.value)}
            className={`${SELECT_CLASS} font-mono`}
          >
            <option value="">Inherit</option>
            {agentKindsWith(agentKind).map((kind) => (
              <option key={kind} value={kind}>
                {kind}
              </option>
            ))}
          </select>
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <label htmlFor={ids.model} className="text-xs font-medium text-slate-300">
            Model
          </label>
          <input
            id={ids.model}
            type="text"
            value={model}
            onChange={(event) => onChange({ ...current, model: event.target.value.trim() === '' ? null : event.target.value })}
            placeholder="Inherit"
            autoComplete="off"
            spellCheck={false}
            className="input-field font-mono"
          />
        </div>
        <div className="flex min-w-0 flex-col gap-1.5">
          <label htmlFor={ids.effort} className="text-xs font-medium text-slate-300">
            Effort
          </label>
          <select
            id={ids.effort}
            value={effort}
            onChange={(event) =>
              onChange({ ...current, effort: isEffortLevel(event.target.value) ? event.target.value : null })
            }
            className={SELECT_CLASS}
          >
            <option value="">Inherit</option>
            {efforts.map((level: EffortLevel) => (
              <option key={level} value={level}>
                {EFFORT_LABELS[level]}
              </option>
            ))}
          </select>
        </div>
      </div>
    </li>
  );
}
