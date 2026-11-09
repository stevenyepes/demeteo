import { modelChoicesFor } from '../lib/dispatchForm';
import type { InstanceDetail } from '../lib/fleet';

interface ModelSelectProps {
  id: string;
  value: string;
  onChange: (model: string) => void;
  /** The instance the run is sent to: only its catalog is offered. */
  instance: InstanceDetail;
  /** The agent in effect, chosen or inherited; blank when there is none. */
  agentKind: string;
  /** What the blank value means here: the project's default, or the step's inherited one. */
  blankLabel: string;
}

const FIELD_CLASS = 'input-field font-mono disabled:cursor-not-allowed disabled:opacity-40';

/**
 * Free text whenever the instance gives no list to choose from: it reports no
 * catalog, as one that predates the field does, or no entry for the agent.
 * With a catalog, a model belongs to one harness, so with no agent in effect
 * there is nothing to choose from. A held value the instance did not list is
 * kept as an option rather than dropped: a project default naming a model the
 * probe missed must still show what will be sent.
 */
export function ModelSelect({ id, value, onChange, instance, agentKind, blankLabel }: ModelSelectProps) {
  const choices = agentKind === '' ? null : modelChoicesFor(instance, agentKind);
  if (instance.harnessCatalog === null || (agentKind !== '' && choices === null)) {
    return (
      <input
        id={id}
        type="text"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={blankLabel}
        autoComplete="off"
        className={FIELD_CLASS}
      />
    );
  }

  const models = choices?.models ?? [];
  const unlisted = value !== '' && !models.some((model) => model.value === value);
  const bundled = agentKind !== '' && choices?.source === 'bundled';
  const hintId = `${id}-hint`;

  return (
    <>
      <select
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        disabled={agentKind === ''}
        aria-describedby={bundled ? hintId : undefined}
        className={FIELD_CLASS}
      >
        <option value="">{agentKind === '' ? 'Pick an agent first' : blankLabel}</option>
        {models.map((model) => (
          <option key={model.value} value={model.value}>
            {model.name === model.value ? model.value : `${model.name} (${model.value})`}
          </option>
        ))}
        {unlisted && <option value={value}>{value} (not reported by this instance)</option>}
      </select>
      {bundled && (
        <p id={hintId} className="text-xs text-slate-500">
          Bundled list: Demeteo's built-in models for {agentKind}, not a list {agentKind} gave on this instance.
        </p>
      )}
    </>
  );
}
