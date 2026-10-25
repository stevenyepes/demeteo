interface PresenceDotProps {
  present: boolean;
}

/** Offline is slate, not ruby: a machine that is switched off has not failed. */
export function PresenceDot({ present }: PresenceDotProps) {
  return (
    <span
      className={`inline-block h-2 w-2 shrink-0 rounded-full ${present ? 'bg-emerald-400 animate-pulse-glow' : 'bg-slate-500'}`}
    >
      <span className="sr-only">{present ? 'Online' : 'Offline'}</span>
    </span>
  );
}
