
export interface CreateZeroNameStepProps {
  projectName: string;
  onChange: (value: string) => void;
}

/** Step 1 — give the project a display name. Mirrors the wizard's
 *  "one decision per screen" rule; the rest of the flow reuses the
 *  name as the default repo slug and feature title. */
export function CreateZeroNameStep(props: CreateZeroNameStepProps) {
  const { projectName, onChange } = props;
  return (
    <div className="space-y-3">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest">
        What do you want to call this project?
      </label>
      <input
        type="text"
        value={projectName}
        onChange={(e) => onChange(e.target.value)}
        placeholder="e.g. billing-service-rust"
        className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white placeholder-slate-600 focus:outline-none focus:border-violet-500/50"
      />
      <p className="text-[11px] text-slate-500 font-mono">
        We'll use this as the display name and as the default repo slug (you can change it on the next step).
      </p>
    </div>
  );
}
