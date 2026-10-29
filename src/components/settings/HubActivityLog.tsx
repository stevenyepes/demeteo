import { Activity } from 'lucide-react';
import type { HubRequestLogEntry, HubRequestOutcome } from '../../lib/hub';
import { logKindLabel, refusalReasonCopy } from '../../lib/hubPresentation';

const OUTCOME_PILL: Readonly<Record<HubRequestOutcome, string>> = {
  accepted: 'bg-emerald-500/10 text-emerald-300 border-emerald-500/30',
  refused: 'bg-ruby-500/10 text-ruby-300 border-ruby-500/30',
  dropped: 'bg-white/5 text-slate-300 border-slate-700/60',
  deferred: 'bg-white/5 text-slate-300 border-slate-700/60',
};

interface HubActivityLogProps {
  entries: HubRequestLogEntry[];
}

export function HubActivityLog({ entries }: HubActivityLogProps) {
  return (
    <div className="panel p-6 space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 flex items-center gap-2">
        <Activity className="w-4 h-4 text-slate-400" /> Hub activity
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        Every request the Hub sent this desktop, newest first, and what this desktop did with it.
      </p>
      {entries.length === 0 ? (
        <p className="text-xs text-slate-500 italic">No request from the Hub has arrived yet.</p>
      ) : (
        <div className="space-y-2">
          {entries.map((entry) => (
            <div
              key={entry.id}
              data-testid="hub-activity-row"
              className="nested-card p-4 flex items-start justify-between gap-4"
            >
              <div className="min-w-0 space-y-1">
                <p className="text-sm text-slate-200">
                  {logKindLabel(entry.kind)}
                  <span className="text-slate-500"> · {entry.scope ?? 'no scope'}</span>
                </p>
                <p className="font-mono text-xs text-slate-400">
                  {new Date(entry.received_at_ms).toLocaleString()}
                </p>
                {entry.reason !== null && (
                  <p className="text-xs text-slate-400">
                    {refusalReasonCopy(entry.reason, entry.scope)}
                  </p>
                )}
              </div>
              <span
                className={`shrink-0 text-[11px] px-2 py-0.5 rounded-full border font-medium ${OUTCOME_PILL[entry.outcome]}`}
              >
                {entry.outcome}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default HubActivityLog;
