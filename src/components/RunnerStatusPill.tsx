import { AlertCircle, Cpu } from 'lucide-react';
import type { ReactNode } from 'react';

import type { RunnerInstallStatus } from '../lib/runner';
import type { RunnerCompatibilityReport } from '../types';

interface RunnerStatusPillProps {
  status: RunnerInstallStatus;
  /** The card's push button label, which the hints below name. */
  pushLabel: string;
}

/** The installed version and what the verdict says about it. Every
 *  comparison is the backend's — nothing here orders two version strings. */
function describe(
  report: RunnerCompatibilityReport | null,
  raw: string,
): { version: string; suffix: ReactNode } {
  switch (report?.verdict) {
    case 'compatible':
      return { version: `${report.version} (${report.channel})`, suffix: null };
    case 'runner_behind':
      return {
        version: `${report.runner} (${report.runner_channel})`,
        suffix: (
          <span className="text-amber-300">
            · update available (<span className="font-mono">{`${report.app} (${report.app_channel})`}</span>)
          </span>
        ),
      };
    case 'runner_ahead':
      return {
        version: `${report.runner} (${report.runner_channel})`,
        suffix: (
          <span className="text-amber-300">
            · newer than Demeteo <span className="font-mono">{`${report.app} (${report.app_channel})`}</span> —
            upgrade Demeteo
          </span>
        ),
      };
    case 'unknown':
      return { version: raw, suffix: <span className="text-slate-400">· version unverified</span> };
    default:
      return { version: raw, suffix: null };
  }
}

export function RunnerStatusPill({ status, pushLabel }: RunnerStatusPillProps) {
  if (!status.version) {
    return (
      <p className="mt-2 text-[11px] text-slate-500 flex items-center gap-1">
        <Cpu className="w-3 h-3" />
        Remote runner not installed — click <span className="text-slate-300">{pushLabel}</span> to
        provision it.
      </p>
    );
  }

  const { version, suffix } = describe(status.compatibility, status.version);
  const isRunning = status.service_active === true;
  const stopped = status.service_active === false;
  const pillTone = isRunning
    ? 'border-emerald-500/20 bg-emerald-500/10 text-emerald-300'
    : stopped
      ? 'border-white/10 bg-white/5 text-slate-300'
      : 'border-white/10 bg-white/5 text-slate-400';
  const dotTone = isRunning
    ? 'bg-emerald-400 shadow-[0_0_8px_rgba(16,185,129,0.7)] animate-pulse'
    : stopped
      ? 'bg-slate-500'
      : 'bg-slate-600';

  return (
    <>
      <div
        title={status.compatibility?.message}
        className={`mt-2 inline-flex flex-wrap items-center gap-1.5 px-2 py-1 rounded-md border text-[11px] font-medium max-w-full ${pillTone}`}
      >
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${dotTone}`} />
        <span>{isRunning ? 'Running' : stopped ? 'Installed, stopped' : 'Installed'}</span>
        <span>·</span>
        <span className="font-mono">{version}</span>
        {suffix}
      </div>
      {isRunning && status.lingering === false && (
        <div className="mt-2 text-[11px] text-amber-300 bg-amber-500/10 border border-amber-500/20 rounded-lg p-2.5 flex items-start gap-2">
          <AlertCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
          <span className="break-words">
            Lingering isn't enabled for this user — the runner will stop when you log out of SSH
            and won't auto-start on reboot. Ask an administrator to run{' '}
            <code className="px-1 py-0.5 rounded bg-white/5 text-amber-200">
              loginctl enable-linger &lt;user&gt;
            </code>{' '}
            on this machine.
          </span>
        </div>
      )}
      {stopped && (
        <p className="mt-1 text-[11px] text-slate-400 flex items-start gap-1">
          <AlertCircle className="w-3 h-3 mt-0.5 shrink-0" />
          <span className="break-words">
            Service is installed but not running. Run{' '}
            <code className="px-1 py-0.5 rounded bg-white/5 text-slate-200">
              systemctl --user start demeteo-runner
            </code>{' '}
            on the remote host, or click <span className="text-slate-200">{pushLabel}</span> to
            re-provision.
          </span>
        </p>
      )}
    </>
  );
}
