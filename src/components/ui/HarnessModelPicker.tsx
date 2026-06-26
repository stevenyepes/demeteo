import { Cpu, Zap, RotateCw, RotateCcw, Check } from 'lucide-react';
import { FieldLabel } from './FieldLabel';

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
  saved = false,
  className = '',
}: HarnessModelPickerProps) {
  const modelEnabled = Boolean(agentKind);

  return (
    <div className={`grid grid-cols-1 sm:grid-cols-[1fr_1fr_auto] gap-3 items-end ${className}`}>
      <div>
        <FieldLabel icon={<Cpu className="w-3 h-3" />}>Harness</FieldLabel>
        <select
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
            disabled={!agentKind && !model}
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
