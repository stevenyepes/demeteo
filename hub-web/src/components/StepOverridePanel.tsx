import { useId } from 'react';

import {
  EFFORT_LABELS,
  EFFORT_SUPPORT,
  isEffortLevel,
  reconcileEffort,
  supportedEffortsFor,
} from '../../../src/lib/effortLevels';
import { isOverridable, type StepOverrideDraft } from '../lib/dispatchForm';
import type { WorkflowGraphNode } from '../lib/fleet';
import { Button } from '../../../src/components/ui/Button';

interface StepOverridePanelProps {
  node: Pick<WorkflowGraphNode, 'id' | 'type' | 'title'>;
  override: StepOverrideDraft;
  /** What the step runs on while its own agent inherits; blank when the form has no default either. */
  defaultAgentKind: string;
  onChange: (next: StepOverrideDraft) => void;
  onReset: () => void;
}

const AGENT_KINDS = Object.keys(EFFORT_SUPPORT);

const SELECT_CLASS =
  'input-field disabled:cursor-not-allowed disabled:opacity-40';

/**
 * Plain controls, not `src/components/ui/HarnessModelPicker.tsx`: that picker
 * lists models from the desktop's agent catalog, which reaches Tauri, while the
 * Hub knows neither an instance's harnesses nor their models. The agent list is
 * therefore the static `EFFORT_SUPPORT` table, and the instance is the judge.
 *
 * A field left on inherit is absent from the draft rather than `''`, so
 * `toStepOverrides` drops a step whose every field inherits.
 */
export function StepOverridePanel({ node, override, defaultAgentKind, onChange, onReset }: StepOverridePanelProps) {
  const agentId = useId();
  const modelId = useId();
  const effortId = useId();

  if (!isOverridable(node.type)) {
    return (
      <div className="nested-card flex flex-col gap-1.5 p-4">
        <h3 className="font-heading text-sm text-slate-100">{node.title}</h3>
        <p className="text-xs text-slate-400">
          This {node.type} step runs no agent, so it has no agent, model or effort to override.
        </p>
      </div>
    );
  }

  const effectiveAgent = override.agentKind || defaultAgentKind;
  const efforts = supportedEffortsFor(effectiveAgent);
  const agentOptions =
    override.agentKind && !AGENT_KINDS.includes(override.agentKind) ? [...AGENT_KINDS, override.agentKind] : AGENT_KINDS;

  const changeAgent = (agentKind: string) => {
    // The same rule as `src/components/StartFeatureModal.tsx`: a model belongs to
    // one harness, and an effort the new harness cannot run would be re-sent anyway.
    const effort = reconcileEffort(override.effort ?? '', supportedEffortsFor(agentKind || defaultAgentKind));
    onChange({ ...(agentKind ? { agentKind } : {}), ...(effort ? { effort } : {}) });
  };

  const changeModel = (model: string) => {
    const { model: _previous, ...rest } = override;
    onChange(model ? { ...rest, model } : rest);
  };

  const changeEffort = (value: string) => {
    const { effort: _previous, ...rest } = override;
    onChange(isEffortLevel(value) ? { ...rest, effort: value } : rest);
  };

  return (
    <div className="nested-card flex flex-col gap-3 p-4">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="font-heading text-sm text-slate-100">{node.title}</h3>
        <Button variant="secondary" size="sm"
          onClick={onReset}>
          Inherit all
        </Button>
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={agentId} className="text-xs font-medium text-slate-300">
          Agent
        </label>
        <select
          id={agentId}
          value={override.agentKind ?? ''}
          onChange={(event) => changeAgent(event.target.value)}
          aria-describedby={`${agentId}-help`}
          className={`${SELECT_CLASS} font-mono`}
        >
          <option value="">Inherit ({defaultAgentKind || 'project default'})</option>
          {agentOptions.map((kind) => (
            <option key={kind} value={kind}>
              {kind}
            </option>
          ))}
        </select>
        <p id={`${agentId}-help`} className="text-xs text-slate-500">
          The instance checks the agent and refuses one it has not registered as harness_not_registered.
        </p>
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={modelId} className="text-xs font-medium text-slate-300">
          Model
        </label>
        <input
          id={modelId}
          type="text"
          value={override.model ?? ''}
          onChange={(event) => changeModel(event.target.value)}
          placeholder="Inherit"
          autoComplete="off"
          className={`${SELECT_CLASS} font-mono`}
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={effortId} className="text-xs font-medium text-slate-300">
          Effort
        </label>
        <select
          id={effortId}
          value={override.effort ?? ''}
          onChange={(event) => changeEffort(event.target.value)}
          disabled={efforts.length === 0}
          aria-describedby={efforts.length === 0 ? `${effortId}-help` : undefined}
          className={SELECT_CLASS}
        >
          <option value="">Inherit</option>
          {efforts.map((level) => (
            <option key={level} value={level}>
              {EFFORT_LABELS[level]}
            </option>
          ))}
        </select>
        {efforts.length === 0 && (
          <p id={`${effortId}-help`} className="text-xs text-slate-500">
            Effort is not supported: {effectiveAgent} has no per-run effort setting, so this step runs at the
            harness's own configured effort.
          </p>
        )}
      </div>
    </div>
  );
}
