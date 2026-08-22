import React from 'react';

interface OptionPillProps {
  selected: boolean;
  /** A capability the harness does not have. The pill dims and the click is a
   *  no-op — never hidden, because the whole point is that the user can see
   *  what this interviewer cannot do. */
  unsupported?: boolean;
  onSelect: () => void;
  children: React.ReactNode;
}

/** The one-of-many chooser the New Discovery modal uses for interviewer,
 *  model and effort. Violet selection: picking the interviewer is the primary
 *  decision on that screen. */
export function OptionPill({
  selected,
  unsupported = false,
  onSelect,
  children,
}: OptionPillProps): React.ReactElement {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      disabled={unsupported}
      onClick={onSelect}
      className={`rounded-lg border px-3.5 py-2 text-xs font-medium transition-colors ${
        selected
          ? 'border-violet-500/40 bg-violet-500/[0.12] text-violet-300 shadow-[0_0_10px_rgba(139,92,246,0.2)]'
          : 'border-white/5 bg-white/[0.02] text-slate-300 hover:border-white/15 hover:text-white'
      } ${unsupported ? 'cursor-not-allowed opacity-35 hover:border-white/5 hover:text-slate-300' : ''}`}
    >
      {children}
    </button>
  );
}

export default OptionPill;
