import { useCallback, useEffect, useRef, useState } from 'react';

import { getFeatureDrift } from '../../lib/featureSync';
import type { FeatureDrift } from '../../types';

/** What a failed read is: not zero commits behind, and not up to date. */
const UNMEASURED: Omit<FeatureDrift, 'checked_at'> = {
  divergence: { behind: null, ahead: null },
  base_ref: '',
  fetched: false,
};

/**
 * How far this feature's branch has fallen behind the base a sync would merge.
 *
 * Read without a fetch on mount and after every sync, which costs two local
 * `git` calls and answers as of the last time anything moved
 * `origin/<base>`; `refresh()` pays the network round trip and is what the
 * user's own press is spent on. The `fetched` flag travels with the number so
 * the chip can say which of the two it is showing rather than implying the
 * expensive one.
 *
 * A read that fails resolves to an unmeasured drift rather than to nothing:
 * the whole point of the signal is that "we could not count it" is a different
 * answer from "there is nothing to pull", and a `null` here would render as the
 * latter.
 */
export function useFeatureDrift(input: {
  featureId: string;
  /** False while the run is still producing commits, and while a sync is in
   *  flight — a count taken mid-merge describes neither side. */
  enabled: boolean;
}): { drift: FeatureDrift | null; refresh: () => void } {
  const { featureId, enabled } = input;
  const [drift, setDrift] = useState<FeatureDrift | null>(null);
  const latest = useRef(0);

  const read = useCallback(
    (fetchFirst: boolean) => {
      latest.current += 1;
      const attempt = latest.current;
      getFeatureDrift(featureId, fetchFirst)
        .then((answer) => {
          if (attempt === latest.current) setDrift(answer);
        })
        .catch(() => {
          if (attempt === latest.current) {
            setDrift({ ...UNMEASURED, checked_at: Date.now() });
          }
        });
    },
    [featureId],
  );

  useEffect(() => {
    if (!enabled) {
      setDrift(null);
      return;
    }
    read(false);
  }, [enabled, read]);

  const refresh = useCallback(() => read(true), [read]);

  return { drift, refresh };
}
