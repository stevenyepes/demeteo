import type { StepExecution } from '../../types';
import {
  ArtifactIcon,
  ARTIFACT_KIND_COLORS,
  ARTIFACT_KIND_LABELS,
  classifyArtifact,
} from '../../lib/artifacts';

export function StepArtifactList({
  step,
  selectedArtifactPath,
  onSelect,
}: {
  step: StepExecution;
  selectedArtifactPath: string | null;
  onSelect: (path: string) => void;
}) {
  // Dedupe: two runner artifacts that share a basename
  // cache to one local file, so `artifact_paths` can
  // carry the same local ref twice — a duplicate here is
  // a duplicate React key below. Keep first occurrence.
  const allPaths = Array.from(new Set(
    step.artifact_paths?.length
      ? step.artifact_paths
      : step.artifact_path ? [step.artifact_path] : []
  ));
  const isAgentStep = step.step_kind === 'agent';
  const visiblePaths = isAgentStep
    ? allPaths.filter(p => classifyArtifact(p).kind === 'markdown')
    : allPaths;
  const hiddenCount = allPaths.length - visiblePaths.length;

  return (
    <>
      {visiblePaths.map((path) => {
        const cls = classifyArtifact(path);
        const Icon = <ArtifactIcon kind={cls.kind} />;
        const labelColor = ARTIFACT_KIND_COLORS[cls.kind];
        return (
          <button
            key={path}
            title={cls.basename}
            onClick={() => onSelect(path)}
            className={`mt-3 w-full text-left text-xs font-mono p-3 rounded border flex items-center gap-3 transition duration-300 ${
              selectedArtifactPath === path
                ? 'bg-violet-950/20 border-violet-500/30 text-violet-300 shadow-[0_0_15px_rgba(139,92,246,0.1)]'
                : 'bg-[#050608] border-white/[0.02] text-slate-400 hover:border-white/10 hover:bg-white/[0.02] hover:text-white cursor-pointer'
            }`}
          >
            <span className={labelColor}>{Icon}</span>
            <span className="truncate flex-1">{cls.basename}</span>
            <span className="text-[9px] uppercase font-bold text-slate-500 shrink-0">
              {ARTIFACT_KIND_LABELS[cls.kind]}
            </span>
          </button>
        );
      })}
      {isAgentStep && hiddenCount > 0 && (
        <div className="mt-3 text-[10px] text-slate-600 font-mono px-1">
          {hiddenCount} file{hiddenCount !== 1 ? 's' : ''} changed · use Browse Code to review
        </div>
      )}
    </>
  );
}
