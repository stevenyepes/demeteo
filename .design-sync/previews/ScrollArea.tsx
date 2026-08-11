import { ScrollArea, StatusBadge, MachineDot } from 'demeteo';

/** A bounded list that scrolls internally rather than growing the page.
 *  The height comes from the caller — ScrollArea only supplies the
 *  overflow, overscroll containment, and thin scrollbar. */
export const SessionList = () => (
  <ScrollArea className="h-56 w-full max-w-sm rounded-xl bg-[#0d0f14] border border-white/5 p-2">
    <div className="flex flex-col gap-1">
      {[
        'demeteo · master',
        'demeteo · feat/ssh-pool',
        'demeteo · feat/windows-parity',
        'demeteo · docs-site',
        'build-01 · shell',
        'build-01 · feat/runner-mirror',
        'gpu-02 · shell',
        'gpu-02 · bench',
        'scratch · notes',
        'scratch · spike',
      ].map((name, i) => (
        <div key={name} className="flex items-center gap-2 px-2.5 py-2 rounded-lg bg-white/[0.02]">
          <MachineDot machineId={i < 4 ? 'local' : 'm-8f21'} machineLabel={i < 4 ? 'local' : 'build-01'} pulse={i < 2} />
          <span className="text-xs text-slate-300 truncate">{name}</span>
        </div>
      ))}
    </div>
  </ScrollArea>
);

/** Inside a flex column — `min-h-0` is what lets it shrink instead of
 *  pushing its siblings off-screen. */
export const InAFlexColumn = () => (
  <div className="flex flex-col h-56 w-full max-w-sm rounded-xl bg-[#0d0f14] border border-white/5 overflow-hidden">
    <div className="px-4 py-3 border-b border-white/5 shrink-0">
      <span className="text-xs font-mono text-slate-400 uppercase tracking-wider">Run events</span>
    </div>
    <ScrollArea className="flex-1 p-2">
      <div className="flex flex-col gap-1.5">
        {[
          ['running', 'Step 1 — decompose'],
          ['completed', 'Step 2 — spec'],
          ['completed', 'Step 3 — implement'],
          ['verifying', 'Step 4 — verify'],
          ['gated', 'Step 5 — review gate'],
          ['pending', 'Step 6 — merge'],
          ['pending', 'Step 7 — publish'],
        ].map(([status, label]) => (
          <div key={label} className="flex items-center gap-2.5 px-2.5 py-2 rounded-lg bg-white/[0.02]">
            <StatusBadge status={status} />
            <span className="text-xs text-slate-300 truncate">{label}</span>
          </div>
        ))}
      </div>
    </ScrollArea>
    <div className="px-4 py-2.5 border-t border-white/5 shrink-0">
      <span className="text-[10px] font-mono text-slate-500">pinned footer stays put</span>
    </div>
  </div>
);
