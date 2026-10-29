import { Lock, ShieldCheck } from 'lucide-react';
import type { HubLocalPolicy, HubScope } from '../../lib/hub';

export type ScopeField = 'read_enabled' | 'spend_enabled' | 'gates_enabled';

const SCOPES: ReadonlyArray<{ scope: HubScope; field: ScopeField; description: string }> = [
  {
    scope: 'read',
    field: 'read_enabled',
    description:
      'Online status, projects by name and remote, runs, step progress, costs, gate summaries.',
  },
  {
    scope: 'spend',
    field: 'spend_enabled',
    description:
      "Start, cancel and re-point runs on this machine's projects. Costs money and agent time until each run finishes or is stopped.",
  },
  {
    scope: 'gates',
    field: 'gates_enabled',
    description: 'Approve, redirect or cancel at a gate from the Hub.',
  },
];

interface HubScopesCardProps {
  policy: HubLocalPolicy | null;
  saving: boolean;
  onToggle: (field: ScopeField) => void;
}

export function HubScopesCard({ policy, saving, onToggle }: HubScopesCardProps) {
  return (
    <div className="glass-panel p-6 rounded-xl space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
        <ShieldCheck className="w-4 h-4 text-violet-400" /> What the Hub may do
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        Each scope is enforced on this machine. No Hub action can turn a toggle on — not a
        request, not a re-pairing, not a setting the Hub reports.
      </p>
      <div className="space-y-2">
        {SCOPES.map(({ scope, field, description }) => {
          const on = policy?.[field] ?? false;
          return (
            <div key={scope} className="nested-card p-4 flex items-start justify-between gap-4">
              <div className="min-w-0 space-y-1">
                <p className="font-mono text-sm text-slate-200">{scope}</p>
                <p className="text-xs text-slate-400 leading-relaxed">{description}</p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={on}
                aria-label={scope}
                onClick={() => onToggle(field)}
                disabled={!policy || saving}
                className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${
                  on ? 'bg-violet-600' : 'bg-white/10'
                }`}
              >
                <span
                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                    on ? 'translate-x-6' : 'translate-x-1'
                  }`}
                />
              </button>
            </div>
          );
        })}
      </div>
      <div className="nested-card p-4 flex items-start gap-3">
        <Lock className="w-4 h-4 text-slate-400 shrink-0 mt-0.5" />
        <div className="space-y-1">
          <p className="text-xs text-slate-200">Never shared, whatever is on</p>
          <p className="text-xs text-slate-400 leading-relaxed">
            Agent transcripts and diffs never leave this instance, nor do provider keys, git
            tokens, harness sign-ins or local paths.
          </p>
        </div>
      </div>
    </div>
  );
}

export default HubScopesCard;
