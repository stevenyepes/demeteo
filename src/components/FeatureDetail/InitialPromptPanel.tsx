/** The prompt the feature was launched with, verbatim. */
export function InitialPromptPanel({ featureDescription }: { featureDescription: string }) {
  return (
    <div className="p-6 bg-[#08090c] border-b border-white/5">
      <div className="max-w-[96ch] flex flex-col gap-2">
        <div className="text-xs text-violet-400 font-bold uppercase tracking-widest flex items-center gap-2">
          Initial Prompt
        </div>
        <div className="p-4 bg-white/[0.02] rounded-xl border border-white/5 text-sm text-slate-300 font-mono whitespace-pre-wrap leading-relaxed shadow-inner max-h-48 overflow-y-auto" title={featureDescription || undefined}>
          {featureDescription
            ? featureDescription
            : <span className="text-slate-500 italic">No initial prompt was recorded for this run.</span>}
        </div>
      </div>
    </div>
  );
}
