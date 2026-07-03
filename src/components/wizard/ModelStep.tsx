import { Loader2, Zap } from 'lucide-react';
import type { CreateProjectStepPayload } from '../../types';

export interface ModelOption {
  value: string;
  name: string;
}

export interface ModelStepProps {
  /** True once both `machineKind/machineId` and `agentKind` are set
   *  — the probe is machine-scoped, so the picker stays disabled
   *  until both inputs are known. */
  enabled: boolean;
  /** True while `getAgentModels(machineId, agentKind)` is in flight. */
  loading: boolean;
  models: ReadonlyArray<ModelOption>;
  value: string;
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'model' }>) => void;
}

/**
 * Step 6 — Model. Depends on the Agent step (and the Machine step
 * for the probe target). Until both are set the picker stays
 * disabled; once probed, the user picks one of the returned models
 * or types a free-form override. Emits the matching
 * `{ step: 'model', model }` payload upward.
 */
export function ModelStep({ enabled, loading, models, value, onSubmit }: ModelStepProps) {
  return (
    <div className="space-y-4" data-testid="wizard-step-model">
      <label
        htmlFor="wizard-model-input"
        className="text-[11px] font-mono text-slate-400 uppercase tracking-widest flex items-center gap-1.5"
      >
        <Zap className="w-3 h-3" /> Model
      </label>

      {!enabled ? (
        <p className="text-xs text-slate-500 font-mono">
          Pick a machine and an agent first — the model probe is
          machine-scoped.
        </p>
      ) : loading ? (
        <div className="flex items-center gap-2 text-xs text-cyan-300 font-mono">
          <Loader2 className="w-3.5 h-3.5 animate-spin" />
          Probing models…
        </div>
      ) : (
        <>
          <select
            value={models.some((m) => m.value === value) ? value : ''}
            onChange={(e) => onSubmit({ step: 'model', model: e.target.value })}
            data-testid="wizard-model-select"
            className="w-full bg-[#08090c] border border-white/10 rounded-lg py-3 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50"
          >
            <option value="">Pick a model…</option>
            {models.map((m) => (
              <option key={m.value} value={m.value}>
                {m.name}
              </option>
            ))}
          </select>

          {/* Free-form override — useful when the probe fails over
              SSH or returns an empty list. Matches the existing
              HarnessModelPicker's "custom" affordance. */}
          <div>
            <label
              htmlFor="wizard-model-input"
              className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1"
            >
              Or type a custom model id
            </label>
            <input
              id="wizard-model-input"
              type="text"
              value={value}
              onChange={(e) => onSubmit({ step: 'model', model: e.target.value })}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && value.trim().length > 0) {
                  e.preventDefault();
                  onSubmit({ step: 'model', model: value.trim() });
                }
              }}
              placeholder="e.g. anthropic/claude-sonnet-4"
              className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-violet-500/50 placeholder-slate-600"
            />
          </div>
        </>
      )}
    </div>
  );
}