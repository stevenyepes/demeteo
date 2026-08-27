import { useCallback, useEffect, useRef, useState } from 'react';

import { getFeatureDrift } from '../../lib/featureSync';
import { unmeasuredDrift } from '../../lib/staleness';
import type { FeatureDrift } from '../../types';

/**
 * How far this feature's branch has fallen behind the base a sync would merge.
 *
 * The read fetches `origin/<base>` first — on mount and after every sync — so
 * it costs a network round trip on opening a finished run. That is the price of
 * the pane's one load-bearing sentence. Nothing else in the app moves that ref
 * for a finished feature, so a count taken off the local one is the *last*
 * fetch's answer to a question the base branch has had every minute since to
 * change: that is how "Nothing to merge" came to sit beside a pull request the
 * forge had already marked conflicted. The project view still counts without
 * fetching and says so — it renders a queue, and a round trip per row is a cost
 * that signal is not worth.
 *
 * So `fetched` means something narrower here than it does there: a `false` on
 * this hook's reading is a fetch that did not land, not a caller that declined
 * to pay for one, and `lib/syncPanel.ts` reads it as exactly that.
 * `refresh()` is the same read again, and stays because a merge landing
 * elsewhere is not something this remounts for.
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
    read(true);
  }, [enabled, read]);

  const refresh = useCallback(() => read(true), [read]);

  return { drift, refresh, refreshing };
}
