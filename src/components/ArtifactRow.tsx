import { ArtifactIcon, ARTIFACT_KIND_COLORS, ARTIFACT_KIND_LABELS, classifyArtifact } from '../lib/artifacts';

/** One selectable artifact row — icon + basename + kind badge. Shared by
 *  `OutputTab` and `GateArtifactPicker` so the row markup can't drift
 *  between the two surfaces that list declared artifacts. */
export function ArtifactRow({
  path,
  selected,
  onSelect,
}: {
  path: string;
  selected: boolean;
  onSelect: () => void;
}) {
  const cls = classifyArtifact(path);
  return (
    <button
      onClick={onSelect}
      className={`flex w-full items-center gap-3 rounded border p-2.5 text-left font-mono text-xs transition ${
        selected
          ? 'border-accent-edge bg-accent-tint text-white'
          : 'border-white/[0.02] bg-[#050608] text-slate-400 hover:border-white/10 hover:bg-white/[0.02] hover:text-white'
      }`}
    >
      <span className={ARTIFACT_KIND_COLORS[cls.kind]}>
        <ArtifactIcon kind={cls.kind} />
      </span>
      <span className="flex-1 truncate">{cls.basename}</span>
      <span className="shrink-0 text-[9px] font-bold text-slate-500">
        {ARTIFACT_KIND_LABELS[cls.kind]}
      </span>
    </button>
  );
}

export default ArtifactRow;
