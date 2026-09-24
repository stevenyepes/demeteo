import { useCallback, useEffect, useState } from 'react';
import {
  AlertTriangle,
  Check,
  Copy,
  Plug,
  Plug2,
  RotateCw,
  Server,
  ShieldCheck,
  Trash2,
} from 'lucide-react';
import { listMcpGrants, revokeMcpGrant, type McpGrantSummary } from '../../lib/mcpGrants';
import {
  getMcpServerStatus,
  setMcpServerEnabled,
  testMcpConnection,
  type McpConnectionTest,
  type McpServerStatus,
} from '../../lib/mcpServer';
import { reportError } from '../../lib/errorBus';
import { McpConnectAgentPanel } from './McpConnectAgentPanel';

/** A grant is flagged as expiring rather than merely active once it has less
 *  than this much life left — the 30-day lifetime (`docs/MCP_INTEGRATION.md` §5) makes a same-day expiry worth calling out before
 *  the client silently loses access. */
const EXPIRY_WARNING_MS = 24 * 60 * 60 * 1000;

/** `{ enabled: true, url: null }` means the toggle is on but the bind failed
 *  (e.g. port collision) — distinct from both a healthy listener and the
 *  user having turned the server off, so it needs its own visual state and
 *  can't be inferred from `enabled` alone. */
type McpServerState = 'listening' | 'not_listening' | 'stopped';

function mcpServerState(enabled: boolean, url: string | null): McpServerState {
  if (!enabled) return 'stopped';
  return url ? 'listening' : 'not_listening';
}

export function McpGrantsTab() {
  const [grants, setGrants] = useState<McpGrantSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [revokingId, setRevokingId] = useState('');

  const [serverEnabled, setServerEnabled] = useState(false);
  const [serverUrl, setServerUrl] = useState<string | null>(null);
  const [serverLoading, setServerLoading] = useState(true);
  const [serverSaving, setServerSaving] = useState(false);
  const [urlCopied, setUrlCopied] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<McpConnectionTest | null>(null);
  const serverState = mcpServerState(serverEnabled, serverUrl);

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

  const applyServerStatus = useCallback((status: McpServerStatus) => {
    setServerEnabled(status.enabled);
    setServerUrl(status.url);
    if (status.enabled && !status.url) {
      reportError(
        'MCP server is enabled but not listening — the configured port may already be in use.',
        { kind: 'conflict' },
      );
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    getMcpServerStatus()
      .then((status) => {
        if (!cancelled) applyServerStatus(status);
      })
      .catch((e) => {
        if (!cancelled) reportError(e);
      })
      .finally(() => {
        if (!cancelled) setServerLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [applyServerStatus]);

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

  const handleToggleServerEnabled = async () => {
    const next = !serverEnabled;
    const previousUrl = serverUrl;
    setServerEnabled(next);
    if (!next) setServerUrl(null);
    setServerSaving(true);
    try {
      await setMcpServerEnabled(next);
      const status = await getMcpServerStatus();
      applyServerStatus(status);
    } catch (e) {
      setServerEnabled(!next);
      setServerUrl(previousUrl);
      reportError(e);
    } finally {
      setServerSaving(false);
    }
  };

  const handleCopyUrl = async () => {
    if (!serverUrl) return;
    try {
      await navigator.clipboard.writeText(serverUrl);
      setUrlCopied(true);
      setTimeout(() => setUrlCopied(false), 1500);
    } catch {
      // Clipboard access denied (e.g. devtools focus) — fail silently.
    }
  };

  const handleTestConnection = async () => {
    setTesting(true);
    setTestResult(null);
    try {
      setTestResult(await testMcpConnection());
    } catch (e) {
      reportError(e);
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="glass-panel p-6 rounded-xl space-y-4">
        <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
          <Server className="w-4 h-4 text-violet-400" /> MCP Server
        </h3>
        <p className="text-xs text-slate-400 leading-relaxed">
          Expose this workspace to external MCP clients over a local HTTP endpoint. Off by
          default; toggling takes effect immediately, no restart required.
        </p>

        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2" role="status">
            <span
              className={`w-2 h-2 rounded-full shrink-0 ${
                serverState === 'listening'
                  ? 'bg-emerald-400 animate-pulse'
                  : serverState === 'not_listening'
                    ? 'bg-amber-400'
                    : 'bg-ruby-400'
              }`}
            />
            <span className="text-xs text-slate-300">
              {serverState === 'listening'
                ? 'Listening'
                : serverState === 'not_listening'
                  ? 'Not Listening'
                  : 'Stopped'}
            </span>
            {serverSaving && <RotateCw className="w-3 h-3 animate-spin text-slate-400" />}
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={serverEnabled}
            aria-label="Enable MCP server"
            onClick={handleToggleServerEnabled}
            disabled={serverSaving || serverLoading}
            className={`relative inline-flex h-6 w-11 shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${
              serverEnabled ? 'bg-cyan-600' : 'bg-white/10'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                serverEnabled ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        </div>

        {serverState === 'listening' && (
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <code className="flex-1 font-mono text-xs text-slate-300 bg-black/40 border border-white/5 rounded-lg px-3 py-2 break-all">
                {serverUrl}
              </code>
              <button
                type="button"
                onClick={handleCopyUrl}
                aria-label="Copy MCP server URL"
                className="shrink-0 p-2 rounded-lg bg-white/5 border border-white/10 hover:bg-violet-500/10 hover:border-violet-500/30 hover:text-violet-400 text-slate-400 transition-all"
              >
                {urlCopied ? (
                  <Check className="w-3.5 h-3.5 text-emerald-400" />
                ) : (
                  <Copy className="w-3.5 h-3.5" />
                )}
              </button>
              <button
                type="button"
                onClick={handleTestConnection}
                disabled={testing}
                className="shrink-0 flex items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-cyan-500/10 hover:border-cyan-500/30 hover:text-cyan-400 text-slate-300 transition-all disabled:opacity-50"
              >
                {testing ? (
                  <RotateCw className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <Plug2 className="w-3.5 h-3.5" />
                )}
                Test connection
              </button>
            </div>
            {testResult?.status === 'reachable' && (
              <p className="text-xs text-emerald-400">
                Reachable — asking clients to sign in, as expected.
              </p>
            )}
            {testResult?.status === 'unreachable' && (
              <p className="text-xs text-ruby-400">Unreachable — {testResult.reason}</p>
            )}
          </div>
        )}

        {serverState === 'not_listening' && (
          <div
            role="alert"
            className="bg-amber-500/10 border border-amber-500/20 p-3 rounded-lg flex items-start gap-3"
          >
            <AlertTriangle className="w-4 h-4 text-amber-400 shrink-0 mt-0.5" />
            <span className="text-xs text-amber-300">
              Enabled, but not listening — the configured port may already be in use.
            </span>
          </div>
        )}
      </div>

      {serverState === 'listening' && serverUrl && <McpConnectAgentPanel serverUrl={serverUrl} />}

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
          <p className="text-xs text-slate-500 italic py-2">
            No active MCP client grants.
            {serverState === 'listening' &&
              ' Waiting for a client to connect — once one requests access, approve it in the window that pops up here.'}
          </p>
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
                      {grant.audience_mismatch ? (
                        <span className="flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded bg-amber-500/10 border border-amber-500/20 text-amber-400 font-mono">
                          <AlertTriangle className="w-2.5 h-2.5" /> Issued for another port
                        </span>
                      ) : expiresSoon ? (
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
    </div>
  );
}

export default McpGrantsTab;
