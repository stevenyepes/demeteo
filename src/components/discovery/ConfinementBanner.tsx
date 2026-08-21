import React from 'react';

interface ConfinementBannerProps {
  /** The harness actually running this interview. */
  agentKind: string;
}

/**
 * What holds the interview to its worktree — stated as intent, with the gap
 * named out loud.
 *
 * **The copy is load-bearing and the second bullet is deliberately brighter
 * than the first.** `docs/PRD_DISCOVERY.md` §4.6 and AGENTS.md §2 both land on
 * the same rule: `PathContainment` carries an `Enforcement` per access class
 * and Windows gets `UNFENCED`, so a read fence is what the harness is asked
 * for, never what the platform guarantees. The write side has no fence at all
 * — the interviewer is simply given no write tools — and saying so is the
 * whole point of the panel. Do not soften either line, and do not collapse
 * them into one.
 *
 * The harness is interpolated rather than written in because a Discovery
 * chooses its own interviewer (§4.5): a banner hard-coded to claude-code would
 * describe a harness that is not running.
 */
export function ConfinementBanner({ agentKind }: ConfinementBannerProps): React.ReactElement {
  return (
    <div
      data-testid="confinement-banner"
      className="mx-4 mt-3 shrink-0 rounded-lg border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed"
    >
      <p className="m-0 text-slate-500">
        Its own worktree, reclaimed while idle. What holds {agentKind} to it:
      </p>
      <ul className="mt-1 mb-0 list-disc pl-4">
        <li className="text-slate-500">
          <span className="font-medium">Reading files</span> — {agentKind}&apos;s own file tools
          refuse to open a file outside this worktree.
        </li>
        <li className="text-amber-200/90">
          <span className="font-medium">Changing files</span> — the interview is given no write
          tools. Nothing below the harness refuses one.
        </li>
      </ul>
    </div>
  );
}

export default ConfinementBanner;
