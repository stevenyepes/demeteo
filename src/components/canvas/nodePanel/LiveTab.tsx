import { Cpu } from 'lucide-react';

import type { AgentStreamStore } from '../../FeatureDetail/useAgentStream';
import { useStreamText, useStreamTruncated } from '../../FeatureDetail/useAgentStream';

/** Stands in for a panel mounted with no run behind it. Module-level so its
 *  identity is stable — `useSyncExternalStore` re-subscribes whenever the store
 *  it is handed changes. */
const NO_STREAM: AgentStreamStore = {
  subscribe: () => () => {},
  read: () => '',
  isTruncated: () => false,
};

/**
 * Live: the running node's agent-stream buffer (same source as the timeline).
 *
 * **The subscription lives here, at the only consumer of it.** `useStreamText`
 * wakes whoever calls it once per animation frame while an agent streams, and
 * this tab is mounted only while it is the selected one — so the other three
 * tabs cost nothing during a stream. One level up in `NodePanel` renders
 * identically and pays that wake on every tab, which for Overview means
 * re-parsing and re-formatting the whole capped run-event feed per frame.
 * Higher still is worse for a second reason: see `StepInspector`'s header.
 *
 * Being the only mount site is also why the truncation line is this tab's to
 * say: `lib/streamBuffer.ts` keeps a bounded tail, and left unsaid, a long
 * turn's last slice reads as everything the agent said.
 */
export function LiveTab({
  streamStore,
  stepExecutionId,
  isStreaming,
}: {
  streamStore?: AgentStreamStore;
  stepExecutionId: string | null;
  isStreaming: boolean;
}) {
  const store = streamStore ?? NO_STREAM;
  const content = useStreamText(store, stepExecutionId).trim();
  const truncated = useStreamTruncated(store, stepExecutionId);
  if (!content) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center text-xs text-slate-500">
        {isStreaming ? (
          <>
            <Cpu className="h-5 w-5 animate-spin text-cyan-400" />
            <span>Waiting for agent output…</span>
          </>
        ) : (
          <span className="font-bold uppercase tracking-wider text-slate-600">
            No live output — this node isn&apos;t running.
          </span>
        )}
      </div>
    );
  }
  return (
    <div className="flex h-full flex-col overflow-hidden px-5 py-4">
      <div className="mb-2 flex shrink-0 items-center gap-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
        {isStreaming && <Cpu className="h-3 w-3 animate-spin text-cyan-400" />}
        Agent reasoning
      </div>
      {truncated && (
        <div className="mb-2 shrink-0 font-mono text-[10px] text-slate-500">
          Earlier output dropped — this is the tail of the turn.
        </div>
      )}
      {/* Newest at the bottom; `flex-col-reverse` keeps it scrolled to live. */}
      <div className="flex min-h-0 flex-1 flex-col-reverse overflow-y-auto rounded-lg border border-cyan-500/20 bg-[#020304] p-3">
        <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-cyan-300/80">
          {content}
        </pre>
      </div>
    </div>
  );
}

export default LiveTab;
