import { GitBranch } from 'lucide-react';
import type { Provider } from '../../types';
import type { CreateProjectStepPayload } from '../../types';

export interface ProviderStepProps {
  providers: ReadonlyArray<Provider>;
  value: string;
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'provider' }>) => void;
}

/**
 * Step 2 — Provider. Two big cards ("GitHub" / "GitLab") that map
 * to the `ProviderInstance.kind` strings the Rust side validates.
 * The list of *configured* providers is supplied by the orchestrator
 * (read from the global project store); this component only renders
 * a picker. It emits the matching `{ step: 'provider',
 * providerId, kind }` payload upward.
 *
 * The wizard still auto-progresses past this step if only one
 * provider is connected (see `CreateProjectWizard`), so the picker
 * stays presentational — it never mutates state on its own.
 */
export function ProviderStep({ providers, value, onSubmit }: ProviderStepProps) {
  const cards: ReadonlyArray<{
    kind: string;
    label: string;
    blurb: string;
    accent: 'violet' | 'cyan';
  }> = [
    { kind: 'github', label: 'GitHub', blurb: 'Personal account or org', accent: 'violet' },
    { kind: 'gitlab', label: 'GitLab', blurb: 'Personal account or group', accent: 'cyan' },
  ];

  return (
    <div className="space-y-4" data-testid="wizard-step-provider">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest">
        Where do you want to host the new repo?
      </label>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {cards.map(({ kind, label, blurb, accent }) => {
          const provider = providers.find((p) => p.type === kind || p.name === kind);
          const connected = Boolean(provider);
          const selected = value === provider?.id;
          return (
            <button
              key={kind}
              type="button"
              disabled={!connected}
              onClick={() => provider && onSubmit({ step: 'provider', providerId: provider.id, kind })}
              data-testid={`wizard-provider-${kind}`}
              className={`relative flex items-start gap-3 p-4 rounded-xl border text-left transition-all ${
                selected
                  ? accent === 'violet'
                    ? 'bg-violet-500/10 border-violet-500/50 text-violet-100 shadow-[0_0_20px_rgba(139,92,246,0.25)]'
                    : 'bg-cyan-500/10 border-cyan-500/50 text-cyan-100 shadow-[0_0_20px_rgba(6,182,212,0.25)]'
                  : connected
                    ? 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
                    : 'bg-black/20 border-white/5 text-slate-600 opacity-60 cursor-not-allowed'
              }`}
            >
              <GitBranch className={`w-5 h-5 mt-0.5 shrink-0 ${selected ? 'text-violet-300' : 'text-slate-400'}`} />
              <div className="min-w-0">
                <div className="text-sm font-semibold">{label}</div>
                <div className="text-[11px] font-mono text-slate-500">{blurb}</div>
                <div className={`mt-2 text-[10px] font-mono uppercase tracking-widest ${
                  connected ? 'text-emerald-300' : 'text-amber-300'
                }`}>
                  {connected ? 'Connected' : 'Not connected'}
                </div>
              </div>
            </button>
          );
        })}
      </div>
      <p className="text-[10px] text-slate-500 font-mono">
        Connect a provider from Settings → Providers if none are listed above.
      </p>
    </div>
  );
}