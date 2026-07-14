import { Gauge, Loader2, Zap } from 'lucide-react';
import { DEFAULT_EFFORT, EFFORT_LABELS, type EffortLevel } from '../../lib/effortLevels';
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
  /** Project-wide default effort. `''` = no project default, which resolves
   *  to the engine default at run time. */
  effort: EffortLevel | '';
  /** The levels the chosen agent declares. Empty (hermes) disables the
   *  control — it has no per-invocation effort knob to set. */
  effortLevels: ReadonlyArray<EffortLevel>;
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'model' }>) => void;
}

/**
 * Step 6 — Model. Depends on the Agent step (and the Machine step
 * for the probe target). Until both are set the picker stays
 * disabled; once probed, the user picks one of the returned models
 * or types a free-form override. Emits the matching
 * `{ step: 'model', model }` payload upward.
 */
export function ModelStep({ enabled, loading, models, value, effort, effortLevels, onSubmit }: ModelStepProps) {
  const effortSupported = effortLevels.length > 0;

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
            onChange={(e) => onSubmit({ step: 'model', model: e.target.value, effort: effort || null })}
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
              onChange={(e) => onSubmit({ step: 'model', model: e.target.value, effort: effort || null })}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && value.trim().length > 0) {
                  e.preventDefault();
                  onSubmit({ step: 'model', model: value.trim(), effort: effort || null });
                }
              }}
              placeholder="e.g. anthropic/claude-sonnet-4"
              className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-violet-500/50 placeholder-slate-600"
            />
          </div>

          {/* Project-wide default reasoning effort. Seeds
              `ProjectSettings.default_effort`; every step inherits it unless
              the workflow, an override or the launch modal says otherwise. */}
          <div>
            <label
              htmlFor="wizard-effort-select"
              className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1 flex items-center gap-1.5"
            >
              <Gauge className="w-3 h-3" /> Default effort
            </label>
            <select
              id="wizard-effort-select"
              value={effort}
              onChange={(e) =>
                onSubmit({
                  step: 'model',
                  model: value,
                  effort: (e.target.value || null) as EffortLevel | null,
                })
              }
              disabled={!effortSupported}
              title={effortSupported ? undefined : 'This agent does not support effort selection'}
              className="w-full bg-[#08090c] border border-white/10 rounded-lg py-2.5 px-3 text-sm text-white focus:outline-none focus:border-violet-500/50 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <option value="">
                {effortSupported ? `Engine default (${EFFORT_LABELS[DEFAULT_EFFORT]})` : 'Not supported'}
              </option>
              {effortLevels.map((l) => (
                <option key={l} value={l}>{EFFORT_LABELS[l]}</option>
              ))}
            </select>
          </div>
        </>
      )}
    </div>
  );
}