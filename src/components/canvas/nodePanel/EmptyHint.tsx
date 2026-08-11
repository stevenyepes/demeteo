import React from 'react';

/** Inline "nothing here" line for a section that has a heading but no rows —
 *  shared so the attempt table and the sequence task list keep reading as the
 *  same surface rather than drifting into two empty states. */
export function EmptyHint({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-white/5 bg-white/[0.01] px-3 py-4 text-center text-xs text-slate-500">
      {children}
    </div>
  );
}

export default EmptyHint;
