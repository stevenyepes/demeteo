import { ArtifactRow } from '../../ArtifactRow';
import { ArtifactViewer } from '../../ArtifactViewer';
import { EnvironmentNotReadyPanel } from '../../EnvironmentNotReadyPanel';
import {
  isBaselineEnvironmentFailure,
  parseEnvironmentFailure,
} from '../../../lib/harnessVerdict';
import type { HarnessBaseline, StepExecution } from '../../../types';

/** Output: declared artifacts (Monaco) + harness/verifier output. */
export function OutputTab({
  step,
  hasOutput,
  artifactPaths,
  hiddenArtifactCount,
  harnessBaseline,
  selectedArtifact,
  onSelectArtifact,
  onOpenEditorForPath,
  onOpenArtifact,
}: {
  step: StepExecution | null;
  hasOutput: boolean;
  artifactPaths: string[];
  /** Declared paths `listStepArtifacts` folded away, summarised rather than
   *  listed. */
  hiddenArtifactCount: number;
  harnessBaseline?: HarnessBaseline | null;
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
  // Two different things end a step and they are not the same claim. The
  // machine failing to run the command carries an action and is not the
  // feature's defect, so it gets the remediation-first panel and the raw text
  // it was composed from is not repeated underneath.
  const environment = isFailed ? parseEnvironmentFailure(step?.error_message) : null;
  // The viewer needs a box to fill, so the tab only scrolls when it is not
  // holding one; with it, its own scroller is the one that moves.
  const inlineViewer = !onOpenArtifact && selectedArtifact !== null;

  if (!hasOutput) {
    return (
      <div className="flex h-full items-center justify-center px-8 text-center text-xs font-bold uppercase tracking-wider text-slate-600">
        No output produced for this node.
      </div>
    );
  }

  return (
    <div
      className={`flex h-full min-h-0 flex-col px-5 py-4 ${inlineViewer ? 'overflow-hidden' : 'overflow-y-auto'}`}
    >
      {environment && (
        <div className="mb-4 shrink-0">
          <EnvironmentNotReadyPanel
            failure={environment}
            atBase={isBaselineEnvironmentFailure(environment, harnessBaseline)}
          />
        </div>
      )}

      {/* Harness / verifier output — the failing-tests / implicated-files surface. */}
      {errorOutput && !environment && (
        <div className="mb-4 shrink-0">
          <div className="mb-2 text-[10px] font-bold uppercase tracking-widest text-slate-500">
            {isFailed ? 'Verifier / harness output' : 'Message'}
          </div>
          <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap break-words rounded-xl border border-rose-500/20 bg-rose-950/10 p-3 font-mono text-[11px] leading-relaxed text-rose-200/90">
            {errorOutput}
          </pre>
        </div>
      )}

      {(artifactPaths.length > 0 || hiddenArtifactCount > 0) && (
        <div className="mb-3 shrink-0 space-y-2">
          <div className="text-[10px] font-bold uppercase tracking-widest text-slate-500">
            Artifacts
          </div>
          {artifactPaths.map((path) => {
            // Nothing stays "selected" when the host owns the preview — the
            // modal is the selection.
            const selected = !onOpenArtifact && selectedArtifact === path;
            return (
              <ArtifactRow
                key={path}
                path={path}
                selected={selected}
                onSelect={() => (onOpenArtifact ? onOpenArtifact(path) : onSelectArtifact(path))}
              />
            );
          })}
          {hiddenArtifactCount > 0 && (
            <div className="px-1 font-mono text-[10px] text-slate-600">
              {hiddenArtifactCount} file{hiddenArtifactCount !== 1 ? 's' : ''} changed · use Browse
              Code to review
            </div>
          )}
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
