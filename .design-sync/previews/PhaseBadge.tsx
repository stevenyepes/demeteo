import { PhaseBadge } from 'demeteo';

/** Every terminal session phase, each with its own icon and tone. */
export const AllPhases = () => (
  <div className="flex flex-col gap-2.5">
    <PhaseBadge phase="connecting" />
    <PhaseBadge phase="running" />
    <PhaseBadge phase="disconnected" />
    <PhaseBadge phase="closed" />
    <PhaseBadge phase="error" />
  </div>
);

/** Laid out as a row, the way a status bar carries it. */
export const InARow = () => (
  <div className="flex flex-wrap items-center gap-4">
    <PhaseBadge phase="connecting" />
    <PhaseBadge phase="running" />
    <PhaseBadge phase="disconnected" />
    <PhaseBadge phase="error" />
  </div>
);

/** In place: the phase closing a terminal's header strip. */
export const InATerminalHeader = () => (
  <div className="flex flex-col gap-2 w-full max-w-md">
    {[
      { title: 'demeteo · master', phase: 'running' },
      { title: 'build-01 · feat/ssh-pool', phase: 'connecting' },
      { title: 'gpu-02 · shell', phase: 'error' },
    ].map((t) => (
      <div
        key={t.title}
        className="flex items-center justify-between gap-4 px-3 py-2.5 rounded-lg bg-black/40 border border-white/5"
      >
        <span className="text-xs font-mono text-slate-300 truncate">{t.title}</span>
        <PhaseBadge phase={t.phase as never} />
      </div>
    ))}
  </div>
);
