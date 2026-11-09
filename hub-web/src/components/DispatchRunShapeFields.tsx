import { useId } from 'react';

import { EFFORT_LABELS, type EffortLevel, isEffortLevel } from '../../../src/lib/effortLevels';
import { agentChoicesFor, effortsFor } from '../lib/dispatchForm';
import type { InstanceDetail } from '../lib/fleet';
import { ModelSelect } from './ModelSelect';

interface DispatchRunShapeFieldsProps {
  /** The instance the run is sent to: its catalog alone says what it can run. */
  instance: InstanceDetail;
  /** Blank inherits the project's default. */
  agentKind: string;
  /** The project's default agent, which a blank `agentKind` inherits; blank when it has none. */
  projectAgentKind: string;
  model: string;
  effort: EffortLevel | '';
  onAgentChange: (agentKind: string) => void;
  onModelChange: (model: string) => void;
  onEffortChange: (effort: EffortLevel | '') => void;
}

const FIELD_CLASS = 'input-field disabled:cursor-not-allowed disabled:opacity-40';

const LABEL_CLASS = 'text-xs font-medium text-slate-300';

export function DispatchRunShapeFields({
  instance,
  agentKind,
  projectAgentKind,
  model,
  effort,
  onAgentChange,
  onModelChange,
  onEffortChange,
}: DispatchRunShapeFieldsProps) {
  const ids = { agent: useId(), model: useId(), effort: useId() };
  const choices = agentChoicesFor(instance);
  const agents = agentKind !== '' && !choices.includes(agentKind) ? [...choices, agentKind] : choices;
  const effectiveAgent = agentKind || projectAgentKind;
  const efforts = effortsFor(instance, agentKind);

  return (
    <div className="grid gap-4 sm:grid-cols-3">
      <div className="flex flex-col gap-1.5">
        <label htmlFor={ids.agent} className={LABEL_CLASS}>
          Default agent
        </label>
        <select
          id={ids.agent}
          value={agentKind}
          onChange={(event) => onAgentChange(event.target.value)}
          className={`${FIELD_CLASS} font-mono`}
        >
          <option value="">Project default</option>
          {agents.map((kind) => (
            <option key={kind} value={kind}>
              {kind}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={ids.model} className={LABEL_CLASS}>
          Model
        </label>
        <ModelSelect
          id={ids.model}
          value={model}
          onChange={onModelChange}
          instance={instance}
          agentKind={effectiveAgent}
          blankLabel="Project default"
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={ids.effort} className={LABEL_CLASS}>
          Effort
        </label>
        <select
          id={ids.effort}
          value={effort}
          onChange={(event) => onEffortChange(isEffortLevel(event.target.value) ? event.target.value : '')}
          disabled={efforts.length === 0}
          className={FIELD_CLASS}
        >
          <option value="">Project default</option>
          {efforts.map((level) => (
            <option key={level} value={level}>
              {EFFORT_LABELS[level]}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
