import { Check, AlertTriangle } from 'lucide-react';
import { BOOTSTRAP_PHASE_ORDER } from '../types';

/** One row of the bootstrap stepper — a normalized view of a
 *  `BootstrapProgressPayload` (from a Tauri `bootstrap_progress` event on the
 *  local path, or a `bootstrap_progress` run-log event on the remote path). */
export interface BootstrapPhaseView {
  id: string;
  label: string;
  status: 'running' | 'completed' | 'failed' | 'skipped' | 'pending' | string;
  detail?: string | null;
}

/** Order a phase map by the canonical {@link BOOTSTRAP_PHASE_ORDER}, appending
 *  any unknown phases in insertion order. */
export function orderBootstrapPhases(
  map: Map<string, BootstrapPhaseView>,
): BootstrapPhaseView[] {
  const known = BOOTSTRAP_PHASE_ORDER.filter((id) => map.has(id)).map(
    (id) => map.get(id)!,
  );
  const extra = [...map.values()].filter(
    (p) => !BOOTSTRAP_PHASE_ORDER.includes(p.id),
  );
  return [...known, ...extra];
}

/**
 * Inline, animated "phase 0" stepper shown at the top of the feature detail
 * timeline while a feature is `bootstrapping` — the window between "Launch"
 * and the first DAG step running. Pulsing dots mark the active phase,
 * completed phases get a green check, a failed phase turns red and shows its
 * error. Cross-fades in via `animate-fade-in`; the parent unmounts it (which
 * cross-fades the real step timeline in) once the first step leaves `pending`.
 *
 * Pure presentation — the parent owns the phase list and updates it from the
 * event stream. Visual language mirrors `ui/CreateZeroBootstrapPanel`.
 */
export function BootstrapStepper({ phases }: { phases: BootstrapPhaseView[] }) {
  if (phases.length === 0) return null;
  const failed = phases.some((p) => p.status === 'failed');

  return (
    <div className="glass-panel border border-white/5 rounded-xl p-4 mb-6 animate-fade-in">
      <h3 className="font-outfit font-semibold text-cyan-400 uppercase tracking-widest text-[11px] mb-3">
        {failed ? 'Bootstrap failed' : 'Bringing your pipeline online'}
      </h3>
      <div className="space-y-2">
        {phases.map((p) => (
          <div key={p.id} className="flex items-start gap-3">
            <StepDot status={p.status} />
            <div className="flex-1 min-w-0">
              <div
                className={`text-sm font-mono leading-5 ${
                  p.status === 'pending' ? 'text-slate-500' : 'text-slate-200'
                }`}
              >
                {p.label}
              </div>
              {p.detail && (
                <div
                  className={`text-[11px] mt-0.5 break-words whitespace-pre-wrap ${
                    p.status === 'failed' ? 'text-ruby-300' : 'text-slate-500'
                  }`}
                >
                  {p.detail}
                </div>
              )}
            </div>
            {p.status === 'completed' && (
              <Check className="w-3.5 h-3.5 text-emerald-400 shrink-0 mt-1" />
            )}
            {p.status === 'failed' && (
              <AlertTriangle className="w-3.5 h-3.5 text-ruby-400 shrink-0 mt-1" />
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

function StepDot({ status }: { status: BootstrapPhaseView['status'] }) {
  const base = 'block w-2 h-2 rounded-full shrink-0 mt-1.5';
  if (status === 'completed') return <span className={`${base} bg-emerald-400`} />;
  if (status === 'running')
    return <span className={`${base} bg-cyan-400 animate-pulse-glow`} />;
  if (status === 'failed') return <span className={`${base} bg-ruby-400`} />;
  return <span className={`${base} bg-slate-600`} />;
}
