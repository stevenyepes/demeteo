/**
 * Artifact preview as an overlay, replacing the run view's right-hand split
 * column. Two things here are deliberate and would otherwise be re-derived
 * wrongly:
 *
 *   1. The root is `ui/Modal`, never a hand-rolled `fixed inset-0`. `<main>` is
 *      `relative z-0` — a stacking context — so an in-place overlay paints
 *      *under* the project rail (see `ui/OverlayPortal`'s doc comment for the
 *      symptom). `Modal` portals to `document.body` and already owns the
 *      backdrop, click-outside and Escape. Only `GateView`'s *card shape* is
 *      copied, not its backdrop root.
 *
 *   2. Every prop handed to `ArtifactViewer` is a plain value or a caller-owned
 *      callback. The viewer is `memo`-wrapped and `FeatureDetail` re-renders on
 *      a 3s poll — an inline arrow or a per-render-recomputed `contentVersion`
 *      means a re-fetch of `artifact_body` and a Monaco remount every tick
 *      (the regression `ArtifactViewer.rerender.test.tsx` pins).
 *
 * Classification comes from `lib/artifacts`, which every run surface now
 * shares; this file must not grow a copy of it.
 */
import { X, ExternalLink } from 'lucide-react';

import { ArtifactViewer } from './ArtifactViewer';
import { Modal } from './ui/Modal';
import {
  ARTIFACT_KIND_COLORS,
  ARTIFACT_KIND_LABELS,
  ArtifactIcon,
  classifyArtifact,
} from '../lib/artifacts';

export interface ArtifactModalProps {
  artifactPath: string;
  /** Raw step_id; the modal humanizes it for the sub-header. */
  stepId?: string | null;
  /** Passed through to `ArtifactViewer`; must be a plain derived string. */
  contentVersion?: string;
  onClose: () => void;
  /** Must be referentially stable (`useCallback`) — `ArtifactViewer` is memoized. */
  onOpenEditorForPath?: (filePath: string) => void;
}

const humanizeStepId = (id: string) =>
  id
    .replace(/^s-/, '')
    .split('-')
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');

export function ArtifactModal({
  artifactPath,
  stepId,
  contentVersion,
  onClose,
  onOpenEditorForPath,
}: ArtifactModalProps) {
  const { kind, basename } = classifyArtifact(artifactPath);

  return (
    <Modal
      onClose={onClose}
      backdropClassName="bg-black/70 p-4"
      // Wider than the app default for the same reason `GateView` documents:
      // an artifact at 42rem wraps every line. `max-h-[85vh]` + a `min-h-0`
      // scroll body is what keeps the card inside the viewport.
      className="w-full max-w-6xl mx-4 bg-[var(--bg-sidebar)] border border-violet-500/30 rounded-2xl shadow-[0_0_50px_rgba(139,92,246,0.15)] overflow-hidden flex flex-col font-sans max-h-[85vh]"
    >
      <div className="p-6 border-b border-white/5 bg-white/[0.01] flex items-center justify-between gap-4">
        <div className="flex items-center gap-3 min-w-0">
          <span
            className={`p-2 rounded-lg bg-white/5 border border-white/10 shrink-0 ${ARTIFACT_KIND_COLORS[kind]}`}
          >
            <ArtifactIcon kind={kind} className="w-5 h-5 shrink-0" />
          </span>
          <div className="min-w-0">
            <h2
              data-testid="artifact-modal-title"
              className="text-lg font-bold font-display text-white tracking-wide truncate"
              title={basename}
            >
              {basename}
            </h2>
            <p className="text-xs text-slate-400 truncate">
              <span className="uppercase tracking-wider font-mono text-[10px] text-slate-500">
                {ARTIFACT_KIND_LABELS[kind]}
              </span>
              {stepId ? <span> · {humanizeStepId(stepId)}</span> : null}
            </p>
          </div>
        </div>
        <button
          onClick={onClose}
          aria-label="Close"
          className="p-1.5 bg-white/5 hover:bg-white/10 rounded-lg text-slate-400 hover:text-white transition shrink-0"
        >
          <X className="w-4 h-4" />
        </button>
      </div>

      <div className="p-6 flex-1 min-h-0 overflow-y-auto">
        {/* A definite height, not a min-height: this box sits inside a
            scrollable body, so a `flex-1` here would resolve against nothing
            and the editor below would inherit a height of zero — GateView
            rendered an empty black panel exactly that way. `vh` keeps it
            proportional to the window instead of a magic pixel count. */}
        <div className="flex h-[62vh] min-h-[240px] flex-col rounded-lg border border-white/5 bg-[var(--bg-well)] p-4 overflow-hidden">
          <ArtifactViewer
            artifactPath={artifactPath}
            contentVersion={contentVersion}
            onOpenEditorForPath={onOpenEditorForPath}
          />
        </div>
      </div>

      <div className="p-6 border-t border-white/5 bg-white/[0.01] flex items-center justify-between gap-4">
        <span className="text-[10px] font-mono text-slate-500 truncate" title={artifactPath}>
          {artifactPath}
        </span>
        {onOpenEditorForPath && (
          <button
            onClick={() => onOpenEditorForPath(artifactPath)}
            className="px-4 py-2 border border-cyan-500/20 hover:border-cyan-500/50 bg-cyan-500/10 hover:bg-cyan-500/20 text-cyan-300 hover:text-white rounded-lg text-xs font-bold transition duration-300 flex items-center gap-1.5 shrink-0"
          >
            <ExternalLink className="w-3.5 h-3.5" /> Open in editor
          </button>
        )}
      </div>
    </Modal>
  );
}
