/**
 * Graph | Timeline toggle for the run column (PRD §6.1).
 *
 * The graph opens first, and a run with no pinned definition falls back to the
 * timeline rather than defaulting to it — settled in UI_REDESIGN_PLAN §7 and
 * PRD §6.1, and spelled in `useRunGraph`'s initialiser. Both surfaces are fed
 * the same steps and the same `run_events` overlay, so this toggle chooses a
 * view and never a source.
 *
 * Placement and spacing are the caller's. The run view seats this in a chrome
 * row beside the density toggle and measures *that row* for the height it has to
 * subtract from the graph box, so a margin or an alignment spelled here would
 * either be invisible to `offsetHeight` or fight the row it sits in.
 *
 * The selected segment's treatment is `SegmentedControl`'s `TONE_CHIP.cyan` and
 * nothing local (UI_REDESIGN_PLAN §5.1). Colours resolve through
 * `lib/runStatus.ts` — audit finding F27 settled that once, and §2 of the plan
 * forbids re-opening it, so a bespoke selected style here would be drift even
 * though it would look fine on its own.
 */
import { List, Network } from 'lucide-react';

import { SegmentedControl, type SegmentedOption } from './ui/SegmentedControl';

export type RunViewMode = 'graph' | 'timeline';

interface RunViewToggleProps {
  mode: RunViewMode;
  onSelect: (mode: RunViewMode) => void;
}

const OPTIONS: readonly SegmentedOption<RunViewMode>[] = [
  { value: 'graph', label: 'Graph', icon: Network },
  { value: 'timeline', label: 'Timeline', icon: List },
];

export function RunViewToggle({ mode, onSelect }: RunViewToggleProps) {
  return (
    <SegmentedControl options={OPTIONS} value={mode} onChange={onSelect} ariaLabel="Run view" />
  );
}
