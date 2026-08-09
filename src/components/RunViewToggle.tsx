/**
 * Graph | Timeline toggle for the run column (PRD §6.1).
 *
 * The graph opens first, and a run with no pinned definition falls back to the
 * timeline rather than defaulting to it — settled in UI_REDESIGN_PLAN §7 and
 * PRD §6.1, and spelled in `useRunGraph`'s initialiser. Both surfaces are fed
 * the same steps and the same `run_events` overlay, so this toggle chooses a
 * view and never a source.
 *
 * It lives in its own file because it is also *chrome* — one of the two
 * elements `useRunColumnLayout` measures to work out how much height is left
 * for the graph box. That is why the positioning
 * classes ride on `SegmentedControl`'s own group element via `className`
 * rather than an enclosing div: `chromeRef` must land on the node whose
 * `offsetHeight` is the height the hook subtracts.
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
  /** `setToggleChromeEl` from `useRunColumnLayout`. */
  chromeRef: (el: HTMLDivElement | null) => void;
}

const OPTIONS: readonly SegmentedOption<RunViewMode>[] = [
  { value: 'graph', label: 'Graph', icon: Network },
  { value: 'timeline', label: 'Timeline', icon: List },
];

export function RunViewToggle({ mode, onSelect, chromeRef }: RunViewToggleProps) {
  return (
    <SegmentedControl
      options={OPTIONS}
      value={mode}
      onChange={onSelect}
      ariaLabel="Run view"
      className="mb-6 self-start"
      ref={chromeRef}
    />
  );
}
