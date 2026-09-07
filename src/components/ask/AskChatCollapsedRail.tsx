import React from 'react';
import { PanelLeftOpen } from 'lucide-react';

interface AskChatCollapsedRailProps {
  onShow: () => void;
  /** A turn is in flight behind the rail. */
  pending: boolean;
}

/**
 * What stands where the chat was while it is hidden — `InterviewCollapsedRail`
 * for the Ask workspace, and for the same reasons: the column is hidden rather
 * than unmounted, so a turn goes on streaming with nothing on screen to say so
 * (hence the emerald dot), and the rail is the only way back, because the
 * control that hides the column lives inside the column.
 */
export function AskChatCollapsedRail({
  onShow,
  pending,
}: AskChatCollapsedRailProps): React.ReactElement {
  return (
    <div className="flex w-10 shrink-0 flex-col items-center gap-3 border-r border-white/5 bg-[rgba(11,13,18,0.4)] py-2.5">
      <button
        type="button"
        onClick={onShow}
        data-testid="ask-chat-show"
        aria-label="Show the chat"
        title="Show the chat"
        className="relative rounded p-1.5 text-slate-400 transition-colors hover:bg-white/5 hover:text-white"
      >
        <PanelLeftOpen className="h-4 w-4" />
        {pending && (
          <span
            data-testid="ask-chat-rail-pulse"
            className="absolute -top-0.5 -right-0.5 h-2 w-2 rounded-full bg-emerald-400 animate-pulse-glow"
          />
        )}
      </button>

      <span className="font-heading text-[10px] font-medium tracking-wider text-slate-500 [writing-mode:vertical-rl]">
        Chat
      </span>
    </div>
  );
}

export default AskChatCollapsedRail;
