import { type Route, routeHref } from '../lib/router';

type Section = 'fleet' | 'runs' | 'dispatch' | 'add-instance' | 'passkeys';

const SECTIONS: readonly { name: Section; label: string }[] = [
  { name: 'fleet', label: 'Fleet' },
  { name: 'runs', label: 'Runs' },
  { name: 'dispatch', label: 'Dispatch' },
  { name: 'add-instance', label: 'Add instance' },
  { name: 'passkeys', label: 'Passkeys' },
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

interface HubNavProps {
  route: Route;
  signingOut: boolean;
  onSignOut: () => void;
}

export function HubNav({ route, signingOut, onSignOut }: HubNavProps) {
  const active = sectionOf(route);
  return (
    <nav aria-label="Primary" className="flex flex-1 flex-wrap items-center gap-1">
      {SECTIONS.map(({ name, label }) => (
        <a
          key={name}
          href={routeHref({ name })}
          aria-current={name === active ? 'page' : undefined}
          className={`rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-cyan-400 ${
            name === active
              ? 'border-accent-edge bg-accent-tint text-white'
              : 'border-transparent text-slate-400 hover:bg-white/5 hover:text-cyan-300 focus-visible:text-cyan-300'
          }`}
        >
          {label}
        </a>
      ))}
      <button
        type="button"
        onClick={onSignOut}
        disabled={signingOut}
        className="ml-auto rounded-lg border border-transparent px-3 py-1.5 text-sm font-medium text-slate-400 transition-colors hover:bg-white/5 hover:text-cyan-300 focus-visible:text-cyan-300 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-cyan-400 disabled:cursor-not-allowed disabled:opacity-40"
      >
        Sign out
      </button>
    </nav>
  );
}
