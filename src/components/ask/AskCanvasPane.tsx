import { useCallback, useEffect, useRef, useState } from 'react';

import { useElapsed } from '../../hooks/useElapsed';
import { exportAskCanvas, listPinnedAskCanvases, pinAskCanvas } from '../../lib/ask';
import { descriptionForNode } from '../../lib/askCanvasCitations';
import { edgesForNode } from '../../lib/askCanvasEdges';
import type { TurnPhase } from '../../lib/askActivity';
import { formatError } from '../../lib/errors';
import type { AskCanvas, AskMessageView, CanvasPathVerdict, PinnedCanvasEntry } from '../../types';
import { ArtifactModal } from '../ArtifactModal';
import { ArtifactRow } from '../ArtifactRow';
import { AskActivityStrip } from './AskActivityStrip';
import { AskCanvasNodeInspector } from './AskCanvasNodeInspector';
import { ROLE_LABEL } from './AskCanvasNode';
import { AskCanvasView } from './AskCanvasView';
import { useStreamedTurn, type AskStreamStore } from './useAskStream';

export interface AskCanvasPaneProps {
  store: AskStreamStore;
  threadId: string;
  projectId: string;
  /** The most recently completed message, or `null` before any turn has
   *  settled. Its `.canvas`, `.prose`, `.id` and `.canvas_paths` are read
   *  here — `.id` is what `heldRef` carries alongside the canvas, so Pin and
   *  Export always name the message the held canvas came from and a later
   *  canvas-free completion cannot make them act on a stale id. Trimming it
   *  collapses that. */
  lastMessage: AskMessageView | null;
  /** `phaseOfStatus(status)` of the thread's current turn, or `null` once it
   *  is idle. The one signal `LiveTurn` cannot supply on its own: `NO_TURN`
   *  is what a thread reads as both before its first turn and after one
   *  settles, so telling those apart needs the status stream, not the fold. */
  phase: TurnPhase | null;
}

/**
 * Canvas when idle-with-canvas, the activity fold while a turn runs, the
 * prior canvas held when a completed turn drew none — never a partial graph.
 *
 * The pinned list and the error banner render in all three states on purpose:
 * this pane is the sole consumer of `listPinnedAskCanvases`, so gating them on
 * a held canvas left a thread's pins on disk with no route to them — reachable
 * on every thread switch, because `AskThreadView` remounts on `key={thread.id}`
 * and `heldRef` starts `null` again. Same reason the banner sits out here:
 * `refreshPinned` fires from a mount effect regardless of state, and its
 * `setError` needs somewhere to land.
 *
 * The Pin/Export toolbar is the one thing that does *not* get that treatment,
 * and the split is deliberate: the list navigates to pins already on disk,
 * whereas both buttons act on `heldRef.current` — so while a turn runs they
 * would pin or download a canvas this pane is deliberately not showing, under
 * a message id nothing on screen names. It is gated on `phase === null` for
 * that reason, not as a leftover of the early return it used to live under.
 *
 * The subscription to the live turn is mounted here, not lifted to
 * `AskThreadView.tsx` — `useAskStream.ts`'s own doc comment and AGENTS.md §3
 * reserve it for the leaf that renders it.
 */
export function AskCanvasPane({ store, threadId, projectId, lastMessage, phase }: AskCanvasPaneProps) {
  const turn = useStreamedTurn(store, threadId);
  const elapsed = useElapsed(turn.startedAt);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [pinned, setPinned] = useState<PinnedCanvasEntry[]>([]);
  const [selectedArtifactPath, setSelectedArtifactPath] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Mutated in render, not in an effect: the point is that this render's
  // output already reflects the latest non-null canvas, not next render's.
  // A `null`-carrying completion leaves it untouched — that is the hold.
  const heldRef = useRef<{
    canvas: AskCanvas;
    answerText: string;
    messageId: string;
    canvasPaths: CanvasPathVerdict[];
  } | null>(null);
  if (lastMessage?.canvas) {
    heldRef.current = {
      canvas: lastMessage.canvas,
      answerText: lastMessage.prose,
      messageId: lastMessage.id,
      canvasPaths: lastMessage.canvas_paths ?? [],
    };
  }

  // `cancelled` follows `ArtifactViewer.tsx`'s guard for the reason spelled
  // out there. `key={thread.id}` already closes the cross-thread case, but
  // the mount effect and a `handlePin`-triggered refresh overlap on a
  // double-click, and the loser would otherwise win the last write.
  const refreshPinned = useCallback(async (isCancelled: () => boolean = () => false) => {
    try {
      const entries = await listPinnedAskCanvases(threadId);
      if (isCancelled()) return;
      setPinned(entries);
      setError(null);
    } catch (cause) {
      if (isCancelled()) return;
      setError(formatError(cause));
    }
  }, [threadId]);

  useEffect(() => {
    let cancelled = false;
    void refreshPinned(() => cancelled);
    return () => {
      cancelled = true;
    };
  }, [refreshPinned]);

  const handleActivate = useCallback((id: string) => {
    setSelectedNodeId((prev) => (prev === id ? null : id));
  }, []);

  // Reads `heldRef.current` at call time rather than closing over the
  // render-time `held` local below — the buttons only render when it's
  // non-null, but the callbacks themselves are created every render (hooks
  // can't be conditional on that).
  const handlePin = useCallback(async () => {
    const current = heldRef.current;
    if (current === null) return;
    setError(null);
    try {
      await pinAskCanvas(threadId, current.messageId);
      await refreshPinned();
    } catch (cause) {
      setError(formatError(cause));
    }
  }, [threadId, refreshPinned]);

  const handleExport = useCallback(async () => {
    const current = heldRef.current;
    if (current === null) return;
    setError(null);
    try {
      const json = await exportAskCanvas(threadId, current.messageId);
      const blob = new Blob([json], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `ask-canvas-${current.messageId}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (cause) {
      setError(formatError(cause));
    }
  }, [threadId]);

  const held = heldRef.current;
  const selectedNode =
    held && selectedNodeId ? held.canvas.nodes.find((n) => n.id === selectedNodeId) : undefined;

  return (
    <div className="flex h-full w-full flex-col">
      {held !== null && phase === null && (
        <div className="flex shrink-0 items-center gap-2 border-b border-white/5 px-3 py-2">
          <button
            type="button"
            onClick={() => void handlePin()}
            className="btn-primary font-mono text-[11px]"
          >
            Pin to Demeteo
          </button>
          <button
            type="button"
            onClick={() => void handleExport()}
            className="btn-secondary font-mono text-[11px]"
          >
            Export
          </button>
        </div>
      )}
      {error && (
        <p role="alert" className="mx-3 mt-2 font-mono text-[11px] text-ruby-200">
          {error}
        </p>
      )}
      {pinned.length > 0 && (
        <div className="flex max-h-40 shrink-0 flex-col gap-2 overflow-y-auto border-b border-white/5 p-2">
          {pinned.map((entry) => (
            <div key={entry.path} className="flex flex-col gap-1">
              {(entry.title !== null || entry.pinned_at !== null) && (
                <div className="flex items-baseline justify-between gap-2 px-0.5">
                  {entry.title !== null && (
                    <span className="truncate text-[11px] text-slate-300">{entry.title}</span>
                  )}
                  {entry.pinned_at !== null && (
                    <span className="shrink-0 font-mono text-[10px] text-slate-500">
                      {new Date(entry.pinned_at).toLocaleString()}
                    </span>
                  )}
                </div>
              )}
              <ArtifactRow
                path={entry.path}
                selected={selectedArtifactPath === entry.path}
                onSelect={() => setSelectedArtifactPath(entry.path)}
              />
            </div>
          ))}
        </div>
      )}
      <div className="min-h-0 flex-1">
        {phase !== null ? (
          <AskActivityStrip turn={turn} elapsedMs={elapsed} />
        ) : held === null ? (
          <div
            data-testid="ask-canvas-placeholder"
            className="flex h-full w-full items-center justify-center font-mono text-[11px] text-slate-500"
          >
            No canvas yet.
          </div>
        ) : (
          <div className="flex h-full min-h-0 w-full">
            <div className="min-w-0 flex-1">
              <AskCanvasView
                canvas={held.canvas}
                answerText={held.answerText}
                canvasPaths={held.canvasPaths}
                selectedNodeId={selectedNodeId}
                onActivate={handleActivate}
              />
            </div>
            {selectedNode && (
              <div className="w-[360px] shrink-0 border-l border-white/5">
                <AskCanvasNodeInspector
                  node={selectedNode}
                  description={descriptionForNode(held.answerText, selectedNode) ?? ROLE_LABEL[selectedNode.role]}
                  {...edgesForNode(held.canvas, selectedNode.id)}
                  threadId={threadId}
                  messageId={held.messageId}
                  projectId={projectId}
                  onDismiss={() => setSelectedNodeId(null)}
                />
              </div>
            )}
          </div>
        )}
      </div>
      {selectedArtifactPath && (
        <ArtifactModal artifactPath={selectedArtifactPath} onClose={() => setSelectedArtifactPath(null)} />
      )}
    </div>
  );
}

export default AskCanvasPane;
