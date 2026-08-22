import { useEffect, useRef, useState } from 'react';

/**
 * `value`, republished at most once every `intervalMs`.
 *
 * The trailing value always arrives: every change schedules the next
 * publication, so the last one is never dropped on the floor the way a plain
 * leading-edge throttle drops it.
 *
 * The point is to give a memoized child a prop that changes on a human's clock
 * rather than the producer's — the caller still re-renders at whatever rate it
 * did before, and only the subtree keyed on this value stops.
 */
export function useThrottledValue<T>(value: T, intervalMs: number): T {
  const [published, setPublished] = useState(value);
  const latest = useRef(value);
  const publishedAt = useRef(Date.now());
  latest.current = value;

  useEffect(() => {
    if (Object.is(value, published)) return;
    const wait = Math.max(0, intervalMs - (Date.now() - publishedAt.current));
    const timer = setTimeout(() => {
      publishedAt.current = Date.now();
      setPublished(latest.current);
    }, wait);
    return () => clearTimeout(timer);
  }, [value, published, intervalMs]);

  return published;
}

export default useThrottledValue;
