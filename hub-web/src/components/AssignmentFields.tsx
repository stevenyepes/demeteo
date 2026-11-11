import { useId } from 'react';

import { AGENTS } from '../../../src/lib/agents';
import { EFFORT_LABELS, EFFORT_LEVELS } from '../../../src/lib/effortLevels';

/** Each field as typed; `''` is unchanged. `assignmentOf` reads it. */
export type AssignmentDraft = { agent: string; model: string; effort: string };

interface AssignmentFieldsProps {
  draft: AssignmentDraft;
  onChange: (draft: AssignmentDraft) => void;
}

/**
 * The model is free text: the Hub reports no harness catalogue, and the
 * desktop's `agentModels.ts` / `agentCatalog.ts` reach Tauri.
 */
export function AssignmentFields({ draft, onChange }: AssignmentFieldsProps) {
  const agentId = useId();
  const modelId = useId();
  const effortId = useId();
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={agentId} className="text-xs font-medium text-slate-300">
          Agent
        </label>
        <select
          id={agentId}
          value={draft.agent}
          onChange={(event) => onChange({ ...draft, agent: event.target.value })}
          className="input-field"
        >
          <option value="">Unchanged</option>
          {Object.values(AGENTS).map((meta) => (
            <option key={meta.kind} value={meta.kind}>
              {meta.label}
            </option>
          ))}
        </select>
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={modelId} className="text-xs font-medium text-slate-300">
          Model
        </label>
        <input
          id={modelId}
          type="text"
          value={draft.model}
          onChange={(event) => onChange({ ...draft, model: event.target.value })}
          placeholder="Unchanged"
          autoComplete="off"
          className="input-field font-mono"
        />
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={effortId} className="text-xs font-medium text-slate-300">
          Effort
        </label>
        <select
          id={effortId}
          value={draft.effort}
          onChange={(event) => onChange({ ...draft, effort: event.target.value })}
          className="input-field"
        >
          <option value="">Unchanged</option>
          {EFFORT_LEVELS.map((level) => (
            <option key={level} value={level}>
              {EFFORT_LABELS[level]}
            </option>
          ))}
        </select>
      </div>
    </>
  );
}
