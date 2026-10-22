import React, { useState } from 'react';
import { Lock } from 'lucide-react';

import type { BaseBranchLock } from '../../lib/discoveryBase';

/** Why the base branch's Change is disabled: one line, with the full ticket
 *  list behind a toggle once there are too many to name in it. */
export function BaseBranchLockNote({ lock }: { lock: BaseBranchLock }): React.ReactElement | null {
  const [open, setOpen] = useState(false);
  if (!lock.locked) return null;

  return (
    <div
      data-testid="discovery-base-locked"
      className="flex flex-col gap-1.5 text-[11px] leading-relaxed text-slate-400"
    >
      <div className="flex min-w-0 items-center gap-2">
        <Lock className="h-3.5 w-3.5 shrink-0 text-slate-500" aria-hidden="true" />
        <span className="min-w-0">{lock.summary}</span>
        {!lock.listed && (
          <button
            type="button"
            aria-expanded={open}
            onClick={() => setOpen((value) => !value)}
            className="shrink-0 rounded px-1 py-0.5 font-medium text-accent-ink transition-colors hover:text-white"
          >
            {open ? 'Hide tickets' : 'Show tickets'}
          </button>
        )}
      </div>
      {open && !lock.listed && (
        <p
          data-testid="discovery-base-locked-tickets"
          className="m-0 pl-5.5 font-mono text-[11px] text-slate-500"
        >
          {lock.tickets.join(' · ')}
        </p>
      )}
    </div>
  );
}

export default BaseBranchLockNote;
