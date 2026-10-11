import { PlaceholderCard } from '../components/PlaceholderCard';
import { routeHref } from '../lib/router';

interface NotFoundProps {
  path: string;
}

export function NotFound({ path }: NotFoundProps) {
  return (
    <PlaceholderCard title="Not found">
      <p>
        No view matches <span className="font-mono text-slate-200 break-all">{path}</span>.
      </p>
      <a
        href={routeHref({ name: 'fleet' })}
        className="mt-4 inline-block text-violet-400 transition-colors hover:text-cyan-300 focus-visible:text-cyan-300"
      >
        Back to Fleet
      </a>
    </PlaceholderCard>
  );
}
