interface TimeoutFieldProps {
  label: string;
  hint: string;
  value: number;
  onChange: (value: number) => void;
}

export function TimeoutField({ label, hint, value, onChange }: TimeoutFieldProps) {
  return (
    <div className="bg-black/40 border border-white/5 rounded-lg p-3">
      <label className="block text-[10px] font-mono text-slate-500 uppercase tracking-widest mb-1.5">
        {label}
      </label>
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={10}
          step={30}
          value={value}
          onChange={e => onChange(Math.max(10, Number(e.target.value) || 0))}
          className="flex-1 bg-black/40 border border-white/10 rounded-lg px-3 py-2 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50"
        />
        <span className="text-[10px] font-mono text-slate-600 uppercase tracking-widest shrink-0">sec</span>
      </div>
      <p className="mt-1 text-[10px] text-slate-500">{hint}</p>
    </div>
  );
}