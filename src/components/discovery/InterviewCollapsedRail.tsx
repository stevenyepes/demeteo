import React from 'react';
import { PanelLeftOpen } from 'lucide-react';

interface InterviewCollapsedRailProps {
  onShow: () => void;
  /** A turn is in flight behind the rail. */
  pending: boolean;
}

/**
 * What stands where the interview was while it is hidden
 * (`DISCOVERY_UI_SPEC.md` §3.4).
 *
 * The column is only hidden, never unmounted, so the turn it is streaming goes
 * on arriving with nothing on screen to say so — hence the emerald dot, the
 * same signal and the same reason as `TopBar`'s collapsed terminal panel. The
 * rail is the sole way back: the header carrying the show/hide control is
 * inside the column it hides.
 */
export function InterviewCollapsedRail({
  onShow,
  pending,
}: InterviewCollapsedRailProps): React.ReactElement {
  return (
    <div className="flex w-10 shrink-0 flex-col items-center gap-3 border-r border-white/5 bg-[#0b0d12]/40 py-2.5">
      <button
        type="button"
        onClick={onShow}
        data-testid="interview-show"
        aria-label="Show the interview"
        title="Show the interview"
        className="relative rounded p-1.5 text-slate-400 transition-colors hover:bg-white/5 hover:text-white"
      >
        <PanelLeftOpen className="h-4 w-4" />
        {pending && (
          <span
            data-testid="interview-rail-pulse"
            className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-emerald-400 animate-pulse-glow"
          />
        )}
      </button>

      <span className="font-heading text-[10px] font-medium tracking-wider text-slate-500 [writing-mode:vertical-rl]">
        Interview
      </span>
    </div>
  );
}

export default InterviewCollapsedRail;
