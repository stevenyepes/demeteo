import React, { useEffect } from 'react';

interface TurnCompleteToastProps {
  title: string;
  /** Discovery title · turn duration. */
  detail: string;
  onDismiss: () => void;
}

/** ~4.5 s, per `DISCOVERY_UI_SPEC.md` §6.6. */
const DISMISS_AFTER_MS = 4500;

/**
 * The completion signal.
 *
 * `docs/PRD_DISCOVERY.md` §4.3 calls it out as load-bearing rather than
 * decoration: leaving mid-interview is the case Discovery is built for, and a
 * multi-minute turn that ends silently forces the user to sit and watch it.
 */
export function TurnCompleteToast({
  title,
  detail,
  onDismiss,
}: TurnCompleteToastProps): React.ReactElement {
  useEffect(() => {
    const timer = window.setTimeout(onDismiss, DISMISS_AFTER_MS);
    return () => window.clearTimeout(timer);
  }, [onDismiss]);

  return (
    <div
      role="status"
      data-testid="turn-complete-toast"
      className="absolute right-6 bottom-6 z-10 flex items-center gap-2.5 rounded-xl border border-emerald-500/25 bg-[#061410]/95 px-4 py-3 shadow-[0_8px_32px_rgba(0,0,0,0.5)]"
    >
      <span
        aria-hidden="true"
        className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-400"
      />
      <div className="min-w-0">
        <p className="m-0 font-heading text-xs font-semibold text-slate-100">{title}</p>
        <p className="m-0 truncate text-[11px] text-slate-400">{detail}</p>
      </div>
    </div>
  );
}

export default TurnCompleteToast;
