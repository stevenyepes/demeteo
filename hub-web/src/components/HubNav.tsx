import { type Route, routeHref } from '../lib/router';

type Section = 'fleet' | 'runs' | 'dispatch' | 'add-instance';

const SECTIONS: readonly { name: Section; label: string }[] = [
  { name: 'fleet', label: 'Fleet' },
  { name: 'runs', label: 'Runs' },
  { name: 'dispatch', label: 'Dispatch' },
  { name: 'add-instance', label: 'Add instance' },
];

/**
 * A detail view lights the list it is reached from: an instance is opened from
 * the fleet, a run from the runs list. No `default` arm, so a route added to
 * `Route` fails to compile here until it is given a section.
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
      return 'add-instance';
    case 'not-found':
      return null;
  }
}

interface HubNavProps {
  route: Route;
}

export function HubNav({ route }: HubNavProps) {
  const active = sectionOf(route);
  return (
    <nav aria-label="Primary" className="flex flex-wrap items-center gap-1">
      {SECTIONS.map(({ name, label }) => (
        <a
          key={name}
          href={routeHref({ name })}
          aria-current={name === active ? 'page' : undefined}
          className={`rounded-lg border px-3 py-1.5 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-cyan-400 ${
            name === active
              ? 'border-violet-500/40 bg-violet-500/10 text-violet-200'
              : 'border-transparent text-slate-400 hover:bg-white/5 hover:text-cyan-300 focus-visible:text-cyan-300'
          }`}
        >
          {label}
        </a>
      ))}
    </nav>
  );
}
