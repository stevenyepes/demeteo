import { useCallback, useEffect, useRef, useState } from 'react';

import { getFeatureDrift } from '../../lib/featureSync';
import { unmeasuredDrift } from '../../lib/staleness';
import type { FeatureDrift } from '../../types';

/**
 * How far this feature's branch has fallen behind the base a sync would merge.
 *
 * Read without a fetch on mount and after every sync, which costs two local
 * `git` calls and answers as of the last time anything moved `origin/<base>`.
 * `refresh()` pays the network round trip, and the chip in `FeatureHeader` is
 * the press that spends it — nothing else in the app fetches that ref outside a
 * run's bootstrap and a sync, so a finished feature with an open pull request
 * has no other way for its count to move at all. The `fetched` flag travels
 * with the number so the chip can say which of the two it is showing rather
 * than implying the expensive one.
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
}): { drift: FeatureDrift | null; refresh: () => void; refreshing: boolean } {
  const { featureId, enabled } = input;
  const [drift, setDrift] = useState<FeatureDrift | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const latest = useRef(0);

  const read = useCallback(
    (fetchFirst: boolean) => {
      latest.current += 1;
      const attempt = latest.current;
      setRefreshing(fetchFirst);
      const land = (answer: FeatureDrift) => {
        if (attempt !== latest.current) return;
        setDrift(answer);
        setRefreshing(false);
      };
      getFeatureDrift(featureId, fetchFirst)
        .then(land)
        .catch(() => land(unmeasuredDrift()));
    },
    [featureId],
  );

  useEffect(() => {
    if (!enabled) {
      // Superseding the in-flight attempt is the whole of this branch's work
      // beyond blanking the chip: a sync starting mid-read would otherwise let
      // the pre-sync count land on top of the cleared state and be rendered
      // for the length of the merge as if it described it.
      latest.current += 1;
      setDrift(null);
      setRefreshing(false);
      return;
    }
    read(false);
  }, [enabled, read]);

  const refresh = useCallback(() => read(true), [read]);

  return { drift, refresh, refreshing };
}
