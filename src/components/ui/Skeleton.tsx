import React from 'react';

export type SkeletonVariant = 'text' | 'block' | 'card';

export interface SkeletonProps {
  variant?: SkeletonVariant;
  /** Lines for `text`, cards for `card`. Ignored by `block`, which is one box. */
  count?: number;
  /** `block` only. A number is read as pixels; a string is used as given, so a
   *  placeholder can inherit the flex box it replaces with `height="100%"`. */
  height?: number | string;
  /** What is loading, in the words a screen reader should hear. */
  label?: string;
  /** Set false inside a container that is already a live region: the
   *  placeholder then renders `aria-hidden`, because two nested `role="status"`
   *  regions announce the same load twice. */
  announce?: boolean;
  className?: string;
}

const BAR = 'animate-skeleton-pulse rounded';

function cssLength(value: number | string): string {
  return typeof value === 'number' ? `${value}px` : value;
}

function unitKeys(count: number): string[] {
  return Array.from({ length: count }, (_, i) => `unit-${i}`);
}

function Lines({ count }: { count: number }): React.ReactElement {
  return (
    <div className="space-y-2.5 w-full">
      {unitKeys(count).map((key, i) => (
        <div
          key={key}
          data-testid="skeleton-line"
          className={`${BAR} h-3 ${count > 1 && i === count - 1 ? 'w-3/4' : 'w-full'}`}
        />
      ))}
    </div>
  );
}

function Card(): React.ReactElement {
  return (
    <div
      data-testid="skeleton-card"
      className="glass-panel rounded-2xl border border-white/5 p-5 space-y-4"
    >
      <div className="flex items-center gap-3">
        <div className={`${BAR} h-4 w-20 rounded-full`} />
        <div className={`${BAR} h-4 flex-1 max-w-[16rem]`} />
      </div>
      <Lines count={2} />
    </div>
  );
}

/**
 * Layout-preserving loading placeholder — the replacement for a centred spinner
 * in a region that owns real estate (UI redesign plan §3.4). A spinner in a
 * flex-1 column swaps the whole subtree out, so returning re-mounts the run
 * graph and re-runs ELK layout; a placeholder that keeps the box lets the
 * caller hold its children's mount point and reads as faster besides.
 *
 * The shimmer is `animate-skeleton-pulse` in `src/App.css`, opacity-only for
 * the GPU reason recorded beside `animate-pulse-glow` there — a screenful of
 * placeholders must cost the compositor nothing but layer opacity.
 */
export function Skeleton({
  variant = 'text',
  count = 1,
  height = '100%',
  label = 'Loading',
  announce = true,
  className = '',
}: SkeletonProps): React.ReactElement {
  const units = Math.max(1, count);

  const body =
    variant === 'block' ? (
      <div
        data-testid="skeleton-block"
        className={`${BAR} w-full rounded-xl`}
        style={{ height: cssLength(height) }}
      />
    ) : variant === 'card' ? (
      <div className="space-y-4">
        {unitKeys(units).map((key) => (
          <Card key={key} />
        ))}
      </div>
    ) : (
      <Lines count={units} />
    );

  const live = announce
    ? { role: 'status' as const, 'aria-busy': true, 'aria-label': label }
    : { 'aria-hidden': true };

  return (
    <div data-testid="skeleton" className={className} {...live}>
      {announce ? <div aria-hidden="true">{body}</div> : body}
    </div>
  );
}

export default Skeleton;
