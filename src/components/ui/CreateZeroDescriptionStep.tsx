
export interface CreateZeroDescriptionStepProps {
  description: string;
  onChange: (value: string) => void;
}

/** Step 7 — the user's free-text description of the feature they
 *  want built. Becomes the `description` field on the launched
 *  Feature. */
export function CreateZeroDescriptionStep(props: CreateZeroDescriptionStepProps) {
  const { description, onChange } = props;
  return (
    <div className="space-y-3">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
        What do you want to build?
      </label>
      <textarea
        value={description}
        onChange={(e) => onChange(e.target.value)}
        rows={8}
        placeholder="Describe the feature. The pipeline will research, draft a spec, implement, review, validate, and ship."
        className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 resize-y"
      />
      <p className="text-[10px] text-slate-500 font-mono">
        {description.trim().length} characters · minimum 8.
      </p>
    </div>
  );
}
