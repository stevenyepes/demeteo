import { useCallback, useRef, useState } from 'react';

import { useElapsed } from '../../hooks/useElapsed';
import { descriptionForNode } from '../../lib/askCanvasCitations';
import { edgesForNode } from '../../lib/askCanvasEdges';
import type { TurnPhase } from '../../lib/askActivity';
import type { AskCanvas, AskMessageView, CanvasPathVerdict } from '../../types';
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
   *  settled. Only its `.canvas`, `.prose`, `.id` and `.canvas_paths` are
   *  read here. */
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
 * The subscription to the live turn is mounted here, not lifted to
 * `AskThreadView.tsx` — `useAskStream.ts`'s own doc comment and AGENTS.md §3
 * reserve it for the leaf that renders it.
 */
export function AskCanvasPane({ store, threadId, projectId, lastMessage, phase }: AskCanvasPaneProps) {
  const turn = useStreamedTurn(store, threadId);
  const elapsed = useElapsed(turn.startedAt);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);

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

  const handleActivate = useCallback((id: string) => {
    setSelectedNodeId((prev) => (prev === id ? null : id));
  }, []);

  if (phase !== null) {
    return <AskActivityStrip turn={turn} elapsedMs={elapsed} />;
  }

  const held = heldRef.current;
  if (held === null) {
    return (
      <div
        data-testid="ask-canvas-placeholder"
        className="flex h-full w-full items-center justify-center font-mono text-[11px] text-slate-500"
      >
        No canvas yet.
      </div>
    );
  }

  const selectedNode = selectedNodeId ? held.canvas.nodes.find((n) => n.id === selectedNodeId) : undefined;

  return (
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
  );
}

export default AskCanvasPane;
