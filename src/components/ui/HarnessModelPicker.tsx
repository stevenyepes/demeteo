import { Cpu, Zap, Gauge, RotateCw, RotateCcw, Check } from 'lucide-react';
import { FieldLabel } from './FieldLabel';
import { EFFORT_LABELS, EFFORT_LEVELS, type EffortLevel } from '../../lib/effortLevels';

export interface ModelOption {
  value: string;
  name: string;
}

interface HarnessModelPickerProps {
  agentKinds: string[];
  models: ModelOption[];
  modelsLoading?: boolean;
  agentKind: string;
  model: string;
  onAgentKindChange: (kind: string) => void;
  onModelChange: (model: string) => void;
  onClear?: () => void;
  agentPlaceholder?: string;
  modelPlaceholder?: string;
  /**
   * The harness this row would run under if its own `agentKind` is left on
   * "inherit" — a workflow-level override, the workflow author's step
   * setting, or the project default. Drives whether the model and effort
   * controls are live: a row that inherits a harness can still pin a model
   * for it. Unset means the row has nothing to inherit.
   */
  inheritedAgentKind?: string;
  /**
   * Reasoning effort. Supplying `onEffortChange` opts the row into the effort
   * control; leaving it out renders the harness+model pair alone, exactly as
   * before.
   */
  effort?: EffortLevel | '';
  onEffortChange?: (effort: EffortLevel | '') => void;
  /**
   * The levels the effective harness accepts, from
   * `AgentCatalogEntry.effort_levels`. An empty list means the agent has no
   * per-invocation effort control (hermes) and the control is disabled with a
   * tooltip saying so, rather than silently offering a level that would be
   * dropped on the floor.
   */
  effortLevels?: readonly EffortLevel[];
  effortPlaceholder?: string;
  saved?: boolean;
  className?: string;
}

export function HarnessModelPicker({
  agentKinds,
  models,
  modelsLoading = false,
  agentKind,
  model,
  onAgentKindChange,
  onModelChange,
  onClear,
  agentPlaceholder = 'Inherit default',
  modelPlaceholder = 'Agent default model',
  inheritedAgentKind = '',
  effort = '',
  onEffortChange,
  effortLevels = EFFORT_LEVELS,
  effortPlaceholder = 'Inherit',
  saved = false,
  className = '',
}: HarnessModelPickerProps) {
  const effectiveAgentKind = agentKind || inheritedAgentKind;
  const modelEnabled = Boolean(effectiveAgentKind);
  const showEffort = Boolean(onEffortChange);
  const effortSupported = effortLevels.length > 0;
  const effortLabel = effectiveAgentKind
    ? effectiveAgentKind.replace(/-/g, ' ')
    : 'this agent';
  const columns = showEffort
    ? 'sm:grid-cols-[1fr_1fr_minmax(7rem,0.6fr)_auto]'
    : 'sm:grid-cols-[1fr_1fr_auto]';

  return (
    <div className={`grid grid-cols-1 ${columns} gap-3 items-end ${className}`}>
      <div>
        <FieldLabel icon={<Cpu className="w-3 h-3" />}>Harness</FieldLabel>
        <select
          aria-label="Harness"
          value={agentKind}
          onChange={(e) => onAgentKindChange(e.target.value)}
          className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 capitalize"
        >
          <option value="">{agentPlaceholder}</option>
          {agentKinds.map((k) => (
            <option key={k} value={k}>{k.replace(/-/g, ' ')}</option>
          ))}
          {agentKind && !agentKinds.includes(agentKind) && (
            <option value={agentKind}>{agentKind.replace(/-/g, ' ')} (unavailable)</option>
          )}
        </select>
      </div>

      <div>
        <FieldLabel icon={<Zap className="w-3 h-3" />}>Model</FieldLabel>
        {modelsLoading ? (
          <div className="w-full bg-[#08090c]/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-slate-400 flex items-center gap-2">
            <RotateCw className="w-3.5 h-3.5 animate-spin text-cyan-400" />
            <span>Probing models…</span>
          </div>
        ) : (
          <select
            aria-label="Model"
            value={model}
            onChange={(e) => onModelChange(e.target.value)}
            disabled={!modelEnabled}
            className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <option value="">{modelEnabled ? modelPlaceholder : 'Pick a harness first'}</option>
            {models.map((m) => (
              <option key={m.value} value={m.value}>{m.name}</option>
            ))}
            {model && !models.some((m) => m.value === model) && (
              <option value={model}>{model} (custom)</option>
            )}
          </select>
        )}
      </div>

      {showEffort && (
        <div>
          <FieldLabel icon={<Gauge className="w-3 h-3" />}>Effort</FieldLabel>
          <select
            aria-label="Effort"
            value={effort}
            onChange={(e) => onEffortChange?.(e.target.value as EffortLevel | '')}
            disabled={!effortSupported}
            title={
              effortSupported
                ? undefined
                : `${effortLabel} does not support effort selection`
            }
            className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 disabled:opacity-40 disabled:cursor-not-allowed capitalize"
          >
            <option value="">
              {effortSupported ? effortPlaceholder : 'Not supported'}
            </option>
            {effortLevels.map((l) => (
              <option key={l} value={l}>{EFFORT_LABELS[l]}</option>
            ))}
          </select>
        </div>
      )}

      {onClear && (
        <div className="flex items-center gap-2 pb-0.5">
          {saved && (
            <span className="flex items-center gap-1 text-[10px] text-emerald-400 font-medium shrink-0 animate-fadeIn">
              <Check className="w-3 h-3" /> Saved
            </span>
          )}
          <button
            type="button"
            onClick={onClear}
            disabled={!agentKind && !model && !effort}
            title="Reset to inherited"
            className="p-2 rounded-lg text-slate-500 hover:text-white bg-white/5 border border-white/10 hover:bg-white/10 transition-all disabled:opacity-25 disabled:cursor-not-allowed shrink-0"
          >
            <RotateCcw className="w-3.5 h-3.5" />
          </button>
        </div>
      )}
    </div>
  );
}
