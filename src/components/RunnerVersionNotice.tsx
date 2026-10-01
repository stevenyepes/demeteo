import { AlertTriangle, RefreshCw, Settings } from 'lucide-react';

import { useNavigation } from '../context';
import { noticeFor, type RunnerNoticeVariant } from '../lib/runnerCompatibility';
import type { RunnerCompatibilityReport } from '../types';

interface RunnerVersionNoticeProps {
  report: RunnerCompatibilityReport | null;
  /** An informational notice is never rendered as an error. */
  variant: RunnerNoticeVariant;
  /** Offered only where the notice blocks: a cached verdict outlives a fix
   *  made from another laptop or the CLI until its TTL expires. */
  onRecheck?: () => void;
  checking?: boolean;
  /** Replaces the default navigation, for a host that has to close itself
   *  first — a portalled modal stays over the page the link lands on. */
  onOpenSettings?: () => void;
}

function versionsOf(report: RunnerCompatibilityReport): { runner: string; app: string } | null {
  if (report.verdict !== 'runner_behind' && report.verdict !== 'runner_ahead') return null;
  return {
    runner: `${report.runner} (${report.runner_channel})`,
    app: `${report.app} (${report.app_channel})`,
  };
}

export function RunnerVersionNotice({
  report,
  variant,
  onRecheck,
  checking = false,
  onOpenSettings,
}: RunnerVersionNoticeProps) {
  const { navigate } = useNavigation();
  if (!report) return null;
  const notice = noticeFor(report, variant);
  if (notice.tone === 'none') return null;

  const alarming = variant === 'blocking' && notice.tone === 'block';
  const versions = versionsOf(report);

  return (
    <div
      role={alarming ? 'alert' : 'status'}
      className={`rounded-xl border bg-[var(--bg-panel)] backdrop-blur-md ${alarming ? 'border-ruby-500/30' : 'border-amber-500/30'}`}
    >
      <div
        className={`flex items-start gap-2.5 px-3 py-2.5 rounded-xl ${alarming ? 'bg-ruby-500/10' : 'bg-amber-500/10'}`}
      >
        <AlertTriangle
          className={`w-4 h-4 mt-0.5 shrink-0 ${alarming ? 'text-ruby-300' : 'text-amber-300'}`}
        />
        <div className="flex-1 min-w-0 flex flex-col gap-1">
          <span className={`text-xs font-semibold ${alarming ? 'text-ruby-200' : 'text-amber-200'}`}>
            {notice.title}
          </span>
          <p className="text-[11px] leading-snug text-slate-300">{notice.body}</p>
          {versions && (
            <p className="text-[10px] text-slate-400">
              Runner <span className="font-mono text-slate-200">{versions.runner}</span>
              {' · '}Demeteo <span className="font-mono text-slate-200">{versions.app}</span>
            </p>
          )}
          {alarming && (
            <p className="text-[11px] leading-snug text-ruby-200/80">
              Launch stays disabled until the runner on this machine matches this Demeteo.
            </p>
          )}
          <div className="flex flex-wrap items-center gap-2 mt-1">
            <button
              type="button"
              onClick={onOpenSettings ?? (() => navigate({ kind: 'settings' }))}
              className="px-2 py-1 rounded-lg border border-violet-500/40 bg-violet-500/10 hover:bg-violet-500/20 text-[11px] font-medium text-violet-200 flex items-center gap-1.5 transition-colors"
            >
              <Settings className="w-3 h-3" />
              Open machine settings
            </button>
            {alarming && onRecheck && (
              <button
                type="button"
                onClick={onRecheck}
                disabled={checking}
                aria-busy={checking}
                className="px-2 py-1 rounded-lg border border-cyan-500/30 bg-cyan-500/[0.07] hover:bg-cyan-500/15 text-[11px] font-medium text-cyan-200 flex items-center gap-1.5 transition-colors disabled:opacity-60 disabled:cursor-not-allowed"
              >
                <RefreshCw className={`w-3 h-3 ${checking ? 'animate-spin' : ''}`} />
                Check again
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
