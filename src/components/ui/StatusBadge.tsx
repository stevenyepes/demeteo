
const DOT_COLORS: Record<string, string> = {
  idle:          'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]',
  active:        'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]',
  completed:     'bg-emerald-500 shadow-[0_0_8px_rgba(16,185,129,0.8)]',
  running:       'bg-cyan-500 shadow-[0_0_8px_rgba(6,182,212,0.8)]',
  bootstrapping: 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)]',
  gated:         'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)]',
  awaiting_gate: 'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)]',
  interrupted:   'bg-amber-500 shadow-[0_0_8px_rgba(245,158,11,0.8)]',
  error:         'bg-ruby-500 shadow-[0_0_8px_rgba(239,68,68,0.8)]',
  failed:        'bg-ruby-500 shadow-[0_0_8px_rgba(239,68,68,0.8)]',
  pending:       'bg-slate-500',
  skipped:       'bg-slate-500',
};

const PILL_COLORS: Record<string, string> = {
  idle:          'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  active:        'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  completed:     'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  running:       'bg-cyan-500/10 text-cyan-400 border-cyan-500/20',
  bootstrapping: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
  gated:         'bg-violet-500/10 text-violet-400 border-violet-500/20',
  awaiting_gate: 'bg-amber-500/10 text-amber-400 border-amber-500/20',
  interrupted:   'bg-amber-500/10 text-amber-400 border-amber-500/20',
  error:         'bg-ruby-500/10 text-ruby-400 border-ruby-500/20',
  failed:        'bg-ruby-500/10 text-ruby-400 border-ruby-500/20',
  pending:       'bg-slate-500/10 text-slate-400 border-slate-500/20',
  skipped:       'bg-slate-500/10 text-slate-400 border-slate-500/20',
};

interface StatusBadgeProps {
  status: string;
  variant?: 'dot' | 'pill';
  label?: string;
  className?: string;
}

export function StatusBadge({ status, variant = 'dot', label, className = '' }: StatusBadgeProps) {
  const normalized = status.toLowerCase();

  if (variant === 'dot') {
    return (
      <div
        className={`w-2 h-2 rounded-full shrink-0 ${DOT_COLORS[normalized] ?? 'bg-slate-600'} ${className}`}
      />
    );
  }

  const display = label ?? status.replace(/_/g, ' ');
  return (
    <span
      className={`inline-flex items-center px-2 py-0.5 rounded text-xs font-medium border capitalize ${PILL_COLORS[normalized] ?? 'bg-slate-500/10 text-slate-400 border-slate-500/20'} ${className}`}
    >
      {display}
    </span>
  );
}
