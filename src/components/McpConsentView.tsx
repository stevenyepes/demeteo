import { useState } from "react";
import { Check, ShieldAlert, X } from "lucide-react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { decideMcpConsent } from "../lib/mcpGrants";
import { reportError } from "../lib/errorBus";

/** Mirrors `DomainEvent::McpConsentRequested` exactly, including field
 *  casing — Tauri event payloads are not auto-camelCased, same convention
 *  as `McpGrantSummary` in `lib/mcpGrants.ts`. `requested_scopes` is
 *  `Vec<Scope>` serialized `rename_all = "snake_case"`, so each entry is
 *  one of `read` | `spend` | `configure` verbatim — the feature brief
 *  requires these exact names on screen, never a resource-shaped rephrasing. */
interface McpConsentRequestedPayload {
  request_id: string;
  client_name: string;
  requested_scopes: string[];
  resource: string;
  redirect_uri: string;
}

/** `adapters/mcp/token.rs::GRANT_LIFETIME_MS` — fixed per grant, not carried
 *  on the event, so this is static copy rather than a value read off the payload. */
const GRANT_DURATION_DAYS = 30;

/** Always-mounted alongside `GateView` (see `App.tsx`) so a consent prompt
 *  renders no matter which screen is open when `raise_main_window` brings
 *  the app forward. Renders nothing until an `mcp_consent_requested` event
 *  arrives. */
export function McpConsentView() {
  const [request, setRequest] = useState<McpConsentRequestedPayload | null>(null);
  const [deciding, setDeciding] = useState(false);

  useTauriEvent<McpConsentRequestedPayload>("mcp_consent_requested", (payload) => {
    setRequest(payload);
  });

  if (!request) return null;

  const decide = async (approve: boolean) => {
    setDeciding(true);
    try {
      await decideMcpConsent(request.request_id, approve);
      setRequest(null);
    } catch (err) {
      reportError(err);
    } finally {
      setDeciding(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-md p-4">
      <div className="w-full max-w-md bg-[#0d0f14] border border-violet-500/30 rounded-2xl shadow-[0_0_50px_rgba(139,92,246,0.15)] overflow-hidden flex flex-col font-sans">
        <div className="p-6 border-b border-white/5 bg-white/[0.01] flex items-center gap-3">
          <span className="p-2 rounded-lg bg-amber-500/10 text-amber-400 border border-amber-500/20 shadow-[0_0_10px_rgba(245,158,11,0.1)]">
            <ShieldAlert className="w-5 h-5" />
          </span>
          <div>
            <h2 className="text-lg font-bold font-heading text-white tracking-wide">
              MCP client requesting access
            </h2>
            <p className="text-xs text-slate-400">Review before granting access to your workspace</p>
          </div>
        </div>

        <div className="p-6 space-y-4">
          <div className="p-4 rounded-lg bg-white/[0.01] border border-white/5 space-y-1">
            <div className="text-[10px] uppercase tracking-wider text-slate-500 font-bold">Client</div>
            <div className="text-white font-semibold font-heading">{request.client_name}</div>
          </div>

          <div className="space-y-1">
            <div className="text-[10px] uppercase tracking-wider text-slate-500 font-bold">Resource</div>
            <div className="text-xs text-slate-300 font-mono break-all">{request.resource}</div>
          </div>

          <div className="space-y-1">
            <div className="text-[10px] uppercase tracking-wider text-slate-500 font-bold">
              Redirect URI
            </div>
            <div className="text-xs text-slate-300 font-mono break-all">{request.redirect_uri}</div>
          </div>

          <div className="space-y-2">
            <div className="text-[10px] uppercase tracking-wider text-slate-500 font-bold">
              Requested scopes
            </div>
            <div className="flex flex-wrap gap-1.5">
              {request.requested_scopes.map((scope) => (
                <span
                  key={scope}
                  className="text-[9px] font-mono px-1.5 py-0.5 rounded bg-violet-500/10 border border-violet-500/20 text-violet-400 uppercase tracking-wider"
                >
                  {scope}
                </span>
              ))}
            </div>
          </div>

          <p className="text-xs text-slate-400 leading-relaxed">
            Approving grants this client access for {GRANT_DURATION_DAYS} days. You can revoke it any
            time from Preferences → MCP.
          </p>
        </div>

        <div className="p-6 border-t border-white/5 bg-white/[0.01] flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={() => decide(false)}
            disabled={deciding}
            className="flex items-center gap-1.5 px-4 py-2 border border-rose-500/20 hover:border-rose-500/50 bg-rose-500/10 hover:bg-rose-500/20 text-rose-400 hover:text-white disabled:opacity-50 disabled:cursor-not-allowed rounded-lg text-xs font-bold transition duration-300"
          >
            <X className="w-3.5 h-3.5" /> Deny
          </button>
          <button
            type="button"
            onClick={() => decide(true)}
            disabled={deciding}
            className="flex items-center gap-1.5 px-5 py-2 bg-emerald-600 hover:bg-emerald-500 hover:shadow-[0_0_20px_rgba(16,185,129,0.5)] disabled:bg-emerald-900/40 disabled:cursor-not-allowed disabled:shadow-none rounded-lg text-xs font-bold text-white transition duration-300 shadow-[0_0_15px_rgba(16,185,129,0.3)]"
          >
            <Check className="w-3.5 h-3.5" /> Approve
          </button>
        </div>
      </div>
    </div>
  );
}

export default McpConsentView;
