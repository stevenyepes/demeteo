import { KeyRound, LogOut, type LucideIcon, Play, Plus, Server, Zap } from 'lucide-react';
import { useId } from 'react';

import RailBadge from '../../../src/components/ui/RailBadge';
import { type Route, routeHref } from '../lib/router';

type Section = 'fleet' | 'runs' | 'dispatch' | 'add-instance' | 'passkeys';

const SECTIONS: readonly { name: Section; label: string; icon: LucideIcon }[] = [
  { name: 'fleet', label: 'Fleet', icon: Server },
  { name: 'runs', label: 'Runs', icon: Zap },
  { name: 'dispatch', label: 'Dispatch', icon: Play },
  { name: 'add-instance', label: 'Add instance', icon: Plus },
  { name: 'passkeys', label: 'Passkeys', icon: KeyRound },
];

/**
 * A detail view lights the list it is reached from: an instance is opened from
 * the fleet, a run from the runs list. Approving a pairing is how an instance
 * gets added, though the page is reached from the desktop and never from a
 * link here. The two auth routes light nothing and are not expected here: the
 * nav exists only signed in, where `App` shows Fleet for them and passes that
 * route instead. No `default` arm, so a route added to `Route` fails to
 * compile here until it is given a section.
 */
function sectionOf(route: Route): Section | null {
  switch (route.name) {
    case 'fleet':
    case 'instance':
      return 'fleet';
    case 'runs':
    case 'run':
      return 'runs';
    case 'dispatch':
      return 'dispatch';
    case 'add-instance':
    case 'pair':
      return 'add-instance';
    case 'passkeys':
      return 'passkeys';
    case 'sign-up':
    case 'sign-in':
    case 'not-found':
      return null;
  }
}

const ITEM =
  'relative flex h-10 w-10 items-center justify-center rounded-lg border transition-colors disabled:cursor-not-allowed disabled:opacity-40';

const IDLE = 'border-transparent text-slate-400 not-disabled:hover:bg-white/5 not-disabled:hover:text-white';

interface HubNavProps {
  route: Route;
  /** Gates waiting across the fleet; `null` until the fleet has been read, and then no badge. */
  needsYou: number | null;
  signingOut: boolean;
  onSignOut: () => void;
}

/**
 * The left rail. Icon-only, so every item carries its label as its accessible
 * name and as a tooltip. The needs-you count rides on Runs, the list a gate is
 * reviewed from.
 */
export function HubNav({ route, needsYou, signingOut, onSignOut }: HubNavProps) {
  const active = sectionOf(route);
  const badgeId = useId();
  const badgeLabel = needsYou === 1 ? '1 gate needs you' : `${needsYou} gates need you`;
  return (
    <nav aria-label="Primary" className="flex flex-1 flex-col items-center gap-2">
      {SECTIONS.map(({ name, label, icon: Icon }) => {
        const badged = name === 'runs' && needsYou !== null && needsYou > 0;
        return (
          <a
            key={name}
            href={routeHref({ name })}
            aria-label={label}
            title={label}
            aria-current={name === active ? 'page' : undefined}
            aria-describedby={badged ? badgeId : undefined}
            className={`${ITEM} ${name === active ? 'border-accent-edge bg-accent-tint text-white' : IDLE}`}
          >
            <Icon aria-hidden="true" className="h-5 w-5" />
            {badged && (
              <>
                <RailBadge tone="amber" count={needsYou} label={badgeLabel} corner="right" />
                <span id={badgeId} className="sr-only">
                  {badgeLabel}
                </span>
              </>
            )}
          </a>
        );
      })}
      <button
        type="button"
        onClick={onSignOut}
        disabled={signingOut}
        aria-label="Sign out"
        title="Sign out"
        className={`${ITEM} ${IDLE} mt-auto`}
      >
        <LogOut aria-hidden="true" className="h-5 w-5" />
      </button>
    </nav>
  );
}
