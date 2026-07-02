import { FolderGit2, Globe, Loader2, Lock } from 'lucide-react';
import type { Provider } from '../../types';
import type { ProviderNamespace } from '../../lib/createProjectWizard';

export interface CreateZeroProviderStepProps {
  projectName: string;
  providers: ReadonlyArray<Provider>;
  providerId: string;
  namespaceId: string;
  repoSlug: string;
  repoPrivate: boolean;
  namespaces: ReadonlyArray<ProviderNamespace>;
  namespacesLoading: boolean;
  onProviderChange: (id: string) => void;
  onNamespaceChange: (id: string) => void;
  onSlugChange: (slug: string) => void;
  onPrivateChange: (isPrivate: boolean) => void;
  /** Pure helper from the main file — exposed so the slug input can
   *  show a friendly per-character validation error inline. */
  validateSlug: (value: string) => string;
}

/**
 * Step 2 — pick a connected provider, the namespace/group to parent
 * the new repo under, the slug, and whether the repo is private.
 * One decision surface per logical row; never overwhelms the user
 * with a single mega-form. Fetches namespaces lazily as the user
 * toggles providers.
 */
export function CreateZeroProviderStep(props: CreateZeroProviderStepProps) {
  const {
    projectName, providers, providerId, namespaceId, repoSlug, repoPrivate,
    namespaces, namespacesLoading,
    onProviderChange, onNamespaceChange, onSlugChange, onPrivateChange, validateSlug,
  } = props;
  const selectedProvider = providers.find((p) => p.id === providerId) ?? null;
  const selectedNamespace = namespaces.find((n) => n.id === namespaceId) ?? null;
  const slugError = validateSlug(repoSlug);
  const slugPlaceholder = projectName.trim().toLowerCase().replace(/[^a-z0-9._-]/g, '-') || 'my-repo';

  return (
    <div className="space-y-4">
      <div>
        <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest mb-2 block">
          Provider
        </label>
        {providers.length === 0 ? (
          <p className="text-xs text-amber-300 font-mono">
            No providers connected. Add one in <span className="underline">Settings → Providers</span>.
          </p>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {providers.map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => onProviderChange(p.id)}
                className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
                  providerId === p.id
                    ? 'bg-violet-500/10 border-violet-500/50 text-violet-100'
                    : 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
                }`}
              >
                <Globe className={`w-4 h-4 ${providerId === p.id ? 'text-violet-300' : 'text-cyan-400'}`} />
                <div className="min-w-0">
                  <div className="font-medium truncate">{p.name}</div>
                  <div className="text-[10px] font-mono text-slate-500 truncate">{p.host}</div>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>

      <div>
        <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest mb-2 block">
          Namespace / group
        </label>
        {namespacesLoading ? (
          <div className="flex items-center gap-2 text-xs text-slate-400 font-mono">
            <Loader2 className="w-3.5 h-3.5 animate-spin text-cyan-400" /> Fetching namespaces…
          </div>
        ) : !providerId ? (
          <p className="text-xs text-slate-500 font-mono">Pick a provider first.</p>
        ) : namespaces.length === 0 ? (
          <p className="text-xs text-ruby-300 font-mono">
            No namespaces found. Verify the provider has `repo` and `read:org` (GitHub) or `api` (GitLab) scopes.
          </p>
        ) : (
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            {namespaces.map((n) => (
              <button
                key={n.id}
                type="button"
                onClick={() => onNamespaceChange(n.id)}
                className={`flex items-center gap-3 p-2.5 rounded-lg border text-left text-sm transition-all ${
                  namespaceId === n.id
                    ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-100'
                    : 'bg-black/30 border-white/10 text-slate-300 hover:border-white/20'
                }`}
              >
                <FolderGit2 className={`w-4 h-4 ${namespaceId === n.id ? 'text-cyan-300' : 'text-slate-500'}`} />
                <div className="min-w-0">
                  <div className="truncate">{n.name}</div>
                  <div className="text-[10px] font-mono text-slate-500">{n.kind}</div>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="grid grid-cols-1 sm:grid-cols-[1fr_auto] gap-3 items-end">
        <div>
          <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest mb-2 block">
            Repository slug
          </label>
          <input
            type="text"
            value={repoSlug}
            onChange={(e) => onSlugChange(e.target.value.toLowerCase().trim())}
            placeholder={slugPlaceholder}
            className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white font-mono placeholder-slate-600 focus:outline-none focus:border-cyan-500/50"
          />
          {slugError && (
            <p className="mt-1 text-[10px] text-amber-300 font-mono">{slugError}</p>
          )}
        </div>
        <div>
          <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest mb-2 block">
            Visibility
          </label>
          <div className="grid grid-cols-2 gap-1.5">
            <button
              type="button"
              onClick={() => onPrivateChange(true)}
              className={`px-3 py-2 rounded-lg text-[11px] font-mono border transition-all flex items-center justify-center gap-1.5 ${
                repoPrivate
                  ? 'bg-violet-500/10 border-violet-500/50 text-violet-200'
                  : 'bg-black/30 border-white/10 text-slate-400 hover:border-white/20'
              }`}
            >
              <Lock className="w-3 h-3" /> Private
            </button>
            <button
              type="button"
              onClick={() => onPrivateChange(false)}
              className={`px-3 py-2 rounded-lg text-[11px] font-mono border transition-all flex items-center justify-center gap-1.5 ${
                !repoPrivate
                  ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-200'
                  : 'bg-black/30 border-white/10 text-slate-400 hover:border-white/20'
              }`}
            >
              <Globe className="w-3 h-3" /> Public
            </button>
          </div>
        </div>
      </div>

      {selectedProvider && selectedNamespace && repoSlug && !slugError && (
        <div className="text-[11px] text-slate-400 font-mono bg-black/30 border border-white/5 rounded-lg px-3 py-2">
          Will create: <span className="text-cyan-300">{selectedNamespace.name}/{repoSlug}</span>
          {repoPrivate ? ' (private)' : ' (public)'} on {selectedProvider.name}
        </div>
      )}
    </div>
  );
}
