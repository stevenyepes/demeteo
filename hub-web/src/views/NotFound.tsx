import { PageHeader } from '../components/PageHeader';
import { routeHref } from '../lib/router';
import { buttonClasses } from '../../../src/components/ui/Button';

interface NotFoundProps {
  path: string;
}

export function NotFound({ path }: NotFoundProps) {
  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Not found" />
      <section className="panel flex max-w-xl flex-col items-start gap-4 p-6">
        <p className="text-sm text-slate-400">
          No view matches <span className="font-mono text-slate-200 break-all">{path}</span>.
        </p>
        <a href={routeHref({ name: 'fleet' })} className={buttonClasses('secondary')}>
          Back to Fleet
        </a>
      </section>
    </div>
  );
}
