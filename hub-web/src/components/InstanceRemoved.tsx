import { Chip } from '../../../src/components/ui/Chip';

/**
 * Slate, never amber or ruby: a removed instance's run needs nobody, and
 * nothing about it failed by being removed.
 */
export function InstanceRemovedChip() {
  return <Chip tone="slate">Instance removed</Chip>;
}

/** Where a removed instance's run would offer a request: no request can reach it. */
export function InstanceRemovedNote({ className = '' }: { className?: string }) {
  return (
    <p className={`text-sm text-slate-300 ${className}`}>
      This run's instance was removed from the Hub, so it can no longer be changed from here.
    </p>
  );
}
