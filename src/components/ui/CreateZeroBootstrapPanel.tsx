import { useEffect, useRef } from 'react';
import { RotateCw, Check, AlertTriangle } from 'lucide-react';

export type BootstrapPhase =
  | 'create_repo'
  | 'create_project'
  | 'bootstrap'
  | 'save_settings'
  | 'done'
  | 'error';

export interface BootstrapPhaseState {
  id: BootstrapPhase;
  label: string;
  status: 'pending' | 'running' | 'done' | 'error';
}

interface CreateZeroBootstrapPanelProps {
  phases: BootstrapPhaseState[];
  /** Live, newline-separated log strip (mono). */
  logs: ReadonlyArray<string>;
  /** Final failure message if the run aborted. */
  errorMessage?: string | null;
  /** Whether the user can retry. Drives the visibility of the retry CTA. */
  canRetry?: boolean;
  onRetry?: () => void;
}

/**
 * Animated, glass-panel stepper shown while the wizard creates the
 * repo, registers the project row, runs the bootstrap clone + strategy
 * detection, and persists the project settings. Pulsing status dots
 * mark the active phase; completed phases get a green check. A mono
 * log strip below mirrors backend stdout so the user can see the
 * progress without leaving the wizard.
 *
 * Pure presentation — the wizard owns the phase list and updates each
 * entry as it transitions through the backend pipeline.
 */
export function CreateZeroBootstrapPanel({
  phases,
  logs,
  errorMessage,
  canRetry = false,
  onRetry,
}: CreateZeroBootstrapPanelProps) {
  const logRef = useRef<HTMLDivElement | null>(null);
  // Auto-scroll the log strip so the latest line stays in view. Cheap
  // because the list is bounded (we cap it in the wizard).
  useEffect(() => {
    if (logRef.current) {
      logRef.current.scrollTop = logRef.current.scrollHeight;
    }
  }, [logs]);

  const isError = phases.some((p) => p.status === 'error');

  return (
    <div className="glass-panel p-6 rounded-xl border-white/10 shadow-2xl flex flex-col gap-4">
      <div>
        <h3 className="font-heading font-semibold text-cyan-400 uppercase tracking-widest text-xs mb-1">
          {isError ? 'Bootstrap failed' : 'Workspace bootstrap'}
        </h3>
        <h2 className="text-xl font-bold text-white">
          {isError ? 'Recover the run' : 'Bringing your new workspace online'}
        </h2>
      </div>

      <div className="space-y-3">
        {phases.map((p) => (
          <div key={p.id} className="flex items-center gap-3">
            <StatusDot status={p.status} />
            <span
              className={`text-sm font-mono flex-1 ${
                p.status === 'pending' ? 'text-slate-500' : 'text-slate-200'
              }`}
            >
              {p.label}
            </span>
            {p.status === 'done' && (
              <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0" />
            )}
            {p.status === 'error' && (
              <AlertTriangle className="w-3.5 h-3.5 text-ruby-400 shrink-0" />
            )}
          </div>
        ))}
      </div>

      <div
        ref={logRef}
        className="w-full max-h-44 overflow-y-auto bg-black/50 border border-white/5 rounded-lg p-3 font-mono text-[11px] leading-relaxed text-slate-300"
        aria-live="polite"
      >
        {logs.length === 0 ? (
          <span className="text-slate-600 italic">Awaiting first event…</span>
        ) : (
          logs.map((line, i) => (
            <div key={i} className="whitespace-pre-wrap break-words">
              {line}
            </div>
          ))
        )}
      </div>

      {errorMessage && (
        <div className="bg-black/40 border border-ruby-500/20 rounded-lg p-3 font-mono text-[11px] text-ruby-200 break-words">
          {errorMessage}
        </div>
      )}

      {canRetry && isError && (
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onRetry}
            className="px-4 py-2 text-xs font-medium bg-ruby-600 hover:bg-ruby-500 text-white rounded-md transition-all flex items-center gap-2"
          >
            <RotateCw className="w-3.5 h-3.5" /> Retry bootstrap
          </button>
        </div>
      )}
    </div>
  );
}

function StatusDot({ status }: { status: BootstrapPhaseState['status'] }) {
  if (status === 'done') {
    return <span className="w-2 h-2 rounded-full bg-emerald-400 shrink-0" />;
  }
  if (status === 'running') {
    return (
      <span className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse-glow shrink-0" />
    );
  }
  if (status === 'error') {
    return <span className="w-2 h-2 rounded-full bg-ruby-400 shrink-0" />;
  }
  return <span className="w-2 h-2 rounded-full bg-slate-600 shrink-0" />;
}
