/**
 * Arrow/Home/End index math for a roving-tabindex row.
 *
 * Extracted because `SegmentedControl` (a radiogroup) and `TabBar` (a tablist)
 * disagree about roles, ARIA and styling but not about what an arrow key means,
 * and a second copy of the key table is how the two rows start answering the
 * same keystroke differently. Vertical arrows are claimed alongside horizontal
 * ones so a row that wraps to two lines still moves; nothing here is aware of
 * orientation.
 *
 * Pure so the wrap-around cases are reachable without a DOM: `null` means the
 * key is not ours and the caller must leave the event alone rather than
 * `preventDefault` it.
 */
export function nextIndexForKey(key: string, from: number, count: number): number | null {
  if (count <= 0) return null;

  const step = STEP[key];
  if (step !== undefined) {
    const origin = from >= 0 ? from : 0;
    return (origin + step + count) % count;
  }
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  return null;
}

const STEP: Record<string, number | undefined> = {
  ArrowRight: 1,
  ArrowDown: 1,
  ArrowLeft: -1,
  ArrowUp: -1,
};
