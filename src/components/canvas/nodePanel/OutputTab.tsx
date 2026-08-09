import { ArtifactViewer } from '../../ArtifactViewer';
import {
  ArtifactIcon,
  ARTIFACT_KIND_COLORS,
  ARTIFACT_KIND_LABELS,
  classifyArtifact,
} from '../../../lib/artifacts';
import type { StepExecution } from '../../../types';

/** Output: declared artifacts (Monaco) + harness/verifier output. */
export function OutputTab({
  step,
  hasOutput,
  artifactPaths,
  selectedArtifact,
  onSelectArtifact,
  onOpenEditorForPath,
  onOpenArtifact,
}: {
  step: StepExecution | null;
  hasOutput: boolean;
  artifactPaths: string[];
  selectedArtifact: string | null;
  onSelectArtifact: (path: string) => void;
  onOpenEditorForPath?: (filePath: string) => void;
  onOpenArtifact?: (artifactPath: string) => void;
}) {
  // Cache-bust the viewer the same way the timeline does: a re-pull can
  // overwrite an artifact at the same path, so key on what changes on a fresh
  // attempt (status/tokens/duration/cost).
  const contentVersion = step
    ? `${step.status}:${step.tokens}:${step.wall_clock_secs}:${step.cost_usd}`
    : undefined;
  const errorOutput = step?.error_message?.trim() || null;
  const isFailed = step?.status === 'failed' || step?.status === 'interrupted';

  if (!hasOutput) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-xs font-bold uppercase tracking-wider text-slate-600">
        No output produced for this node.
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden px-5 py-4">
      {/* Harness / verifier output — the failing-tests / implicated-files surface. */}
      {errorOutput && (
        <div className="mb-4 shrink-0">
          <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
            {isFailed ? 'Verifier / harness output' : 'Message'}
          </div>
          <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border border-rose-500/20 bg-rose-950/10 p-3 font-mono text-[11px] leading-relaxed text-rose-200/90">
            {errorOutput}
          </pre>
        </div>
      )}

      {artifactPaths.length > 0 && (
        <div className="mb-3 shrink-0 space-y-2">
          <div className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
            Artifacts
          </div>
          {artifactPaths.map((path) => {
            const cls = classifyArtifact(path);
            // Nothing stays "selected" when the host owns the preview — the
            // modal is the selection.
            const selected = !onOpenArtifact && selectedArtifact === path;
            return (
              <button
                key={path}
                onClick={() => (onOpenArtifact ? onOpenArtifact(path) : onSelectArtifact(path))}
                className={`flex w-full items-center gap-3 rounded border p-2.5 text-left font-mono text-xs transition ${
                  selected
                    ? 'border-violet-500/30 bg-violet-950/20 text-violet-300 shadow-[0_0_15px_rgba(139,92,246,0.1)]'
                    : 'border-white/[0.02] bg-[#050608] text-slate-400 hover:border-white/10 hover:bg-white/[0.02] hover:text-white'
                }`}
              >
                <span className={ARTIFACT_KIND_COLORS[cls.kind]}>
                  <ArtifactIcon kind={cls.kind} />
                </span>
                <span className="flex-1 truncate">{cls.basename}</span>
                <span className="shrink-0 text-[9px] font-bold uppercase text-slate-500">
                  {ARTIFACT_KIND_LABELS[cls.kind]}
                </span>
              </button>
            );
          })}
        </div>
      )}

      {/* Selected artifact body — only when the host hasn't taken the preview
          over via `onOpenArtifact`. */}
      {!onOpenArtifact && selectedArtifact && (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <ArtifactViewer
            artifactPath={selectedArtifact}
            contentVersion={contentVersion}
            onOpenEditorForPath={onOpenEditorForPath}
          />
        </div>
      )}
    </div>
  );
}

export default OutputTab;
