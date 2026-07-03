import { FolderGit2, Loader2 } from 'lucide-react';
import type { ProviderNamespace } from '../../lib/createProjectWizard';
import type { CreateProjectStepPayload } from '../../types';

export interface GroupStepProps {
  /** Authenticated user's namespaces for the selected provider. */
  namespaces: ReadonlyArray<ProviderNamespace>;
  /** True while the namespace list is being fetched. */
  loading: boolean;
  value: string;
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'group' }>) => void;
}

/**
 * Step 3 — Group / namespace. Shows the namespaces the wizard has
 * already fetched for the selected provider, plus a small "type
 * your own" text input as a fallback (in case the user wants a
 * group the orchestrator couldn't enumerate). The first option in
 * the list is always the personal namespace; orgs / groups follow.
 */
export function GroupStep({ namespaces, loading, value, onSubmit }: GroupStepProps) {
  const selected = namespaces.find((n) => n.id === value);

  return (
    <div className="space-y-4" data-testid="wizard-step-group">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest">
        Namespace / group
      </label>

      {loading ? (
        <div className="flex items-center gap-2 text-xs text-slate-400 font-mono">
          <Loader2 className="w-3.5 h-3.5 animate-spin text-cyan-400" />
          Fetching namespaces…
        </div>
      ) : namespaces.length === 0 ? (
        <p className="text-xs text-amber-300 font-mono">
          No namespaces found. Verify the provider has the required scopes
          (GitHub: <code>repo</code>, <code>read:org</code>; GitLab: <code>api</code>).
        </p>
      ) : (
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          {namespaces.map((n) => (
            <button
              key={n.id}
              type="button"
              onClick={() => onSubmit({ step: 'group', namespaceId: n.id, kind: n.kind, name: n.name })}
              data-testid={`wizard-namespace-${n.id}`}
              className={`flex items-center gap-3 p-2.5 rounded-lg border text-left text-sm transition-all ${
                value === n.id
                  ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-100'
                  : 'bg-black/30 border-white/10 text-slate-300 hover:border-white/20'
              }`}
            >
              <FolderGit2 className={`w-4 h-4 ${value === n.id ? 'text-cyan-300' : 'text-slate-500'}`} />
              <div className="min-w-0">
                <div className="truncate">{n.name}</div>
                <div className="text-[10px] font-mono text-slate-500">{n.kind}</div>
              </div>
            </button>
          ))}
        </div>
      )}

      {/* Manual override — useful for group paths the fetch
          endpoint didn't enumerate. Empty submit is ignored. */}
      <div>
        <label
          htmlFor="wizard-namespace-manual"
          className="text-[10px] font-mono text-slate-500 uppercase tracking-widest block mb-1"
        >
          Or type a namespace id
        </label>
        <input
          id="wizard-namespace-manual"
          type="text"
          placeholder="e.g. my-org"
          defaultValue={selected?.id ?? ''}
          onKeyDown={(e) => {
            if (e.key !== 'Enter') return;
            const v = (e.target as HTMLInputElement).value.trim();
            if (!v) return;
            e.preventDefault();
            const match = namespaces.find((n) => n.id === v);
            onSubmit({
              step: 'group',
              namespaceId: v,
              kind: match?.kind ?? 'org',
              name: match?.name ?? v,
            });
          }}
          className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
        />
      </div>
    </div>
  );
}