import { useId } from 'react';

import type { PairingScope } from '../lib/pairing';

/** What each scope allows, as `docs/HUB.md` §7 states it. */
const ALLOWS: Record<PairingScope, string> = {
  read: 'This instance sends its events and snapshots to the Hub.',
  spend: 'The Hub may start, cancel and re-point runs that spend money.',
  gates: 'The Hub may relay a gate decision to this instance.',
};

interface PairingScopesProps {
  requested: PairingScope[];
  /**
   * What the user took away, not what they kept: the scopes approved are the
   * requested ones less these, so no state a page can reach holds a scope the
   * machine did not ask for.
   */
  withheld: PairingScope[];
  onToggle: (scope: PairingScope, checked: boolean) => void;
  disabled: boolean;
}

export function PairingScopes({ requested, withheld, onToggle, disabled }: PairingScopesProps) {
  const id = useId();
  return (
    <fieldset className="flex flex-col">
      <legend className="text-xs font-medium text-slate-400">Scopes requested</legend>
      <p className="mt-1 mb-2 text-xs text-slate-400">
        Uncheck any you do not want to grant. Nothing the machine did not ask for can be added here.
      </p>
      <div className="flex flex-col divide-y divide-hairline rounded-md border border-hairline">
        {requested.map((scope) => (
          <div key={scope} className="flex items-start gap-3 px-4 py-3">
            <input
              id={`${id}-${scope}`}
              type="checkbox"
              checked={!withheld.includes(scope)}
              onChange={(event) => onToggle(scope, event.target.checked)}
              disabled={disabled}
              aria-describedby={`${id}-${scope}-allows`}
              className="mt-0.5 accent-accent disabled:cursor-not-allowed disabled:opacity-40"
            />
            <div className="flex min-w-0 flex-col gap-0.5">
              <label htmlFor={`${id}-${scope}`} className="font-mono text-sm text-slate-200">
                {scope}
              </label>
              <span id={`${id}-${scope}-allows`} className="text-xs text-slate-400">
                {ALLOWS[scope]}
              </span>
            </div>
          </div>
        ))}
      </div>
    </fieldset>
  );
}
