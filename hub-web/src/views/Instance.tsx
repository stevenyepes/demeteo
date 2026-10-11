import { PlaceholderCard } from '../components/PlaceholderCard';

interface InstanceProps {
  instanceId: string;
}

export function Instance({ instanceId }: InstanceProps) {
  return (
    <PlaceholderCard title="Instance">
      <p>
        Details for instance <span className="font-mono text-slate-200 break-all">{instanceId}</span> will appear here.
      </p>
    </PlaceholderCard>
  );
}
