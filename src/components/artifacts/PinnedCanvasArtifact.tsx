/**
 * Renders a `<message_id>.canvas.json` artifact — an Ask Canvas pinned to the
 * artifact store — as the same grid the live `AskCanvasPane` draws, wrapped in
 * the header strip every `ArtifactViewer` branch shares.
 *
 * Node selection is owned here rather than passed in. `AskCanvasNode` gives
 * every node with a non-null `path` `cursor-pointer` and an `onClick`, so a
 * viewer that hands it a constant `selectedNodeId` renders an affordance that
 * takes a click and does nothing; the toggle below is the one `AskCanvasPane`
 * uses, so a reopened pin selects the way the pane it came from does.
 *
 * The snapshot's JSON parsing and its malformed-body fallback stay in
 * `ArtifactViewer` — this component's contract is an already-decoded snapshot.
 */
import { useState } from 'react';

import type { AskCanvas, CanvasPathVerdict } from '../../types';
import { AskCanvasView } from '../ask/AskCanvasView';

export interface PinnedCanvasArtifactProps {
  /** Path of the artifact on disk; only its basename is displayed. */
  artifactPath: string | null;
  /** The raw artifact body, for "Copy Complete Output". */
  body: string;
  canvas: AskCanvas;
  canvasPaths: CanvasPathVerdict[];
  checkedCommitSha: string | null;
}

export function PinnedCanvasArtifact({
  artifactPath,
  body,
  canvas,
  canvasPaths,
  checkedCommitSha,
}: PinnedCanvasArtifactProps) {
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const unresolved = canvasPaths.filter((p) => !p.resolved);

  return (
    <div className="flex-1 min-h-0 min-w-0 rounded-xl border border-white/5 overflow-hidden shadow-lg bg-[#050608]/85 flex flex-col">
      <div className="bg-white/[0.02] px-4 py-2 border-b border-white/5 flex justify-between items-center text-[10px] uppercase font-bold text-slate-500 tracking-wider shrink-0">
        <span className="flex items-center gap-2 truncate">
          <span className="text-violet-400/80 font-mono normal-case tracking-tight truncate" title={artifactPath ?? ''}>
            {artifactPath ? artifactPath.split('/').pop() : ''}
          </span>
          <span className="text-slate-600">·</span>
          <span>Pinned Canvas</span>
        </span>
        <button
          onClick={() => navigator.clipboard.writeText(body)}
          className="hover:text-white transition duration-150 shrink-0"
        >
          Copy Complete Output
        </button>
      </div>
      <div className="px-4 py-2 border-b border-white/5 text-[11px] text-slate-400 font-mono space-y-1 shrink-0">
        <div>
          Checked commit: <span className="text-slate-200">{checkedCommitSha ?? 'unknown'}</span>
        </div>
        {unresolved.length > 0 && (
          <div className="text-amber-400 space-y-0.5">
            <div>Unresolved paths:</div>
            {unresolved.map((p) => (
              <div key={`${p.node_id}:${p.path}`} className="text-slate-300 pl-2">
                {p.path}
              </div>
            ))}
          </div>
        )}
      </div>
      <div className="flex-1 min-h-0 min-w-0">
        <AskCanvasView
          canvas={canvas}
          answerText=""
          selectedNodeId={selectedNodeId}
          onActivate={(id) => setSelectedNodeId((prev) => (prev === id ? null : id))}
        />
      </div>
    </div>
  );
}
