import React from 'react';
import { Check } from 'lucide-react';

export interface CreateZeroStepDescriptor {
  id: string;
  label: string;
}

interface CreateZeroStepHeaderProps {
  steps: CreateZeroStepDescriptor[];
  /** id of the currently active step. */
  activeId: string;
  /** ids of completed steps (shown with a green check). */
  completedIds: ReadonlyArray<string>;
  className?: string;
}

/**
 * Horizontal step indicator for the Create-From-Zero wizard. One
 * pill per logical decision; the active one glows violet, completed
 * ones show a green check, pending ones stay muted. Renders as a row
 * of nodes + connectors that flexes to fit the container; collapses
 * to a single label on very narrow widths.
 */
export function CreateZeroStepHeader({
  steps,
  activeId,
  completedIds,
  className = '',
}: CreateZeroStepHeaderProps) {
  const completed = new Set(completedIds);
  return (
    <div className={`w-full overflow-x-auto ${className}`}>
      <ol className="flex items-center gap-2 min-w-fit py-1">
        {steps.map((s, idx) => {
          const isActive = s.id === activeId;
          const isDone = completed.has(s.id) && !isActive;
          return (
            <React.Fragment key={s.id}>
              <li
                className={`flex items-center gap-2 px-3 py-1.5 rounded-full border text-[11px] font-mono uppercase tracking-wider transition-all duration-300 shrink-0 ${
                  isActive
                    ? 'bg-violet-500/10 border-violet-500/50 text-violet-200 shadow-[0_0_15px_rgba(139,92,246,0.25)]'
                    : isDone
                      ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-200'
                      : 'bg-black/30 border-white/10 text-slate-500'
                }`}
              >
                <span
                  className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-semibold shrink-0 ${
                    isActive
                      ? 'bg-violet-500/30 text-violet-100'
                      : isDone
                        ? 'bg-emerald-500/30 text-emerald-100'
                        : 'bg-white/5 text-slate-500'
                  }`}
                >
                  {isDone ? <Check className="w-3 h-3" /> : idx + 1}
                </span>
                <span className="whitespace-nowrap">{s.label}</span>
              </li>
              {idx < steps.length - 1 && (
                <span
                  className={`h-px w-6 shrink-0 transition-colors duration-300 ${
                    isDone ? 'bg-emerald-500/40' : 'bg-white/10'
                  }`}
                />
              )}
            </React.Fragment>
          );
        })}
      </ol>
    </div>
  );
}
