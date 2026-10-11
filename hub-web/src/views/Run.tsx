import { PlaceholderCard } from '../components/PlaceholderCard';

interface RunProps {
  instanceId: string;
  featureId: string;
}

export function Run({ instanceId, featureId }: RunProps) {
  return (
    <PlaceholderCard title="Run">
      <p>The run's progress will appear here.</p>
      <dl className="nested-card mt-4 grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 p-4">
        <dt>Instance</dt>
        <dd className="font-mono text-slate-200 break-all">{instanceId}</dd>
        <dt>Feature</dt>
        <dd className="font-mono text-slate-200 break-all">{featureId}</dd>
      </dl>
    </PlaceholderCard>
  );
}
