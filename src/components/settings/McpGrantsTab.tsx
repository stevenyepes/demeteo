import { useEffect, useState } from 'react';
import { AlertTriangle, Plug, RotateCw, ShieldCheck, Trash2 } from 'lucide-react';
import { listMcpGrants, revokeMcpGrant, type McpGrantSummary } from '../../lib/mcpGrants';
import { reportError } from '../../lib/errorBus';

/** A grant is flagged as expiring rather than merely active once it has less
 *  than this much life left — the 30-day lifetime (§7 Open Question 3 of
 *  implementation-spec.md) makes a same-day expiry worth calling out before
 *  the client silently loses access. */
const EXPIRY_WARNING_MS = 24 * 60 * 60 * 1000;

export function McpGrantsTab() {
  const [grants, setGrants] = useState<McpGrantSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [revokingId, setRevokingId] = useState('');

  useEffect(() => {
    let cancelled = false;
    listMcpGrants()
      .then((result) => {
        if (!cancelled) setGrants(result);
      })
      .catch((e) => {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleRevoke = async (grantId: string) => {
    setRevokingId(grantId);
    try {
      await revokeMcpGrant(grantId);
      setGrants((prev) => prev.filter((g) => g.id !== grantId));
    } catch (e) {
      reportError(e);
    } finally {
      setRevokingId('');
    }
  };

  return (
    <div className="glass-panel p-6 rounded-xl space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
        <Plug className="w-4 h-4 text-violet-400" /> MCP Client Grants
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        External MCP clients that have completed the consent flow and hold an active access
        grant to this workspace. Revoking a grant takes effect on its next request — no
        restart required.
      </p>

      {loadError && (
        <div className="bg-ruby-500/10 border border-ruby-500/30 p-3 rounded-lg flex items-start gap-3">
          <AlertTriangle className="w-4 h-4 text-ruby-400 shrink-0 mt-0.5" />
          <span className="text-sm text-ruby-200">{loadError}</span>
        </div>
      )}

      {isLoading ? (
        <div className="flex items-center justify-center py-8">
          <RotateCw className="w-5 h-5 text-cyan-400 animate-spin" />
        </div>
      ) : grants.length === 0 ? (
        <p className="text-xs text-slate-500 italic py-2">No active MCP client grants.</p>
      ) : (
        <div className="space-y-2">
          {grants.map((grant) => {
            const expiresSoon = grant.expires_at - Date.now() < EXPIRY_WARNING_MS;
            return (
              <div
                key={grant.id}
                className="flex items-start gap-3 p-3 border border-white/5 rounded-lg bg-black/20"
              >
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm font-semibold text-white font-heading">
                      {grant.client_name}
                    </span>
                    {expiresSoon ? (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-ruby-500/10 border border-ruby-500/20 text-ruby-400 font-mono">
                        <AlertTriangle className="w-2.5 h-2.5" /> Expiring soon
                      </span>
                    ) : (
                      <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-emerald-500/10 border border-emerald-500/20 text-emerald-400 font-mono">
                        <ShieldCheck className="w-2.5 h-2.5" /> Active
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 flex-wrap mt-1.5">
                    {grant.scopes.map((scope) => (
                      <span
                        key={scope}
                        className="text-[9px] font-mono px-1.5 py-0.5 rounded bg-violet-500/10 border border-violet-500/20 text-violet-400 uppercase tracking-wider"
                      >
                        {scope}
                      </span>
                    ))}
                  </div>
                  <p className={`text-xs mt-1.5 ${expiresSoon ? 'text-ruby-400' : 'text-slate-400'}`}>
                    Expires {new Date(grant.expires_at).toLocaleString()}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => handleRevoke(grant.id)}
                  disabled={revokingId === grant.id}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-ruby-500/10 hover:border-ruby-500/30 hover:text-ruby-400 text-slate-300 transition-all disabled:opacity-50 shrink-0"
                >
                  {revokingId === grant.id ? (
                    <RotateCw className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <Trash2 className="w-3.5 h-3.5" />
                  )}
                  Revoke
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

export default McpGrantsTab;
