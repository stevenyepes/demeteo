import { Rows2, Rows4 } from 'lucide-react';

import type { Density } from '../../lib/density';
import { SegmentedControl, type SegmentedOption } from './SegmentedControl';

export interface DensityToggleProps {
  value: Density;
  onChange: (value: Density) => void;
  /** Names the list this controls. Required because two surfaces carry one of
   *  these, and a control whose accessible name says "timeline" while it sizes
   *  the pipeline list is worse than an unnamed one. */
  ariaLabel: string;
  className?: string;
}

/**
 * Comfortable/compact picker for a long list (`docs/UI_REDESIGN_PLAN.md` §3.7).
 *
 * Value and setter are props: the choice is persisted in Phase 6, through
 * `get_app_session`/`set_app_session` rather than component state, so owning it
 * here would be a second home to unpick.
 *
 * Generic because the run timeline and the project view's pipeline list both
 * offer one, and §5.1 spends its length on what happens when a second copy of a
 * control gets written instead of the first being generalized.
 */
export function DensityToggle({
  value,
  onChange,
  ariaLabel,
  className = '',
}: DensityToggleProps): React.ReactElement {
  return (
    <SegmentedControl
      options={OPTIONS}
      value={value}
      onChange={onChange}
      ariaLabel={ariaLabel}
      size="sm"
      className={className}
    />
  );
}

const OPTIONS: readonly SegmentedOption<Density>[] = [
  { value: 'comfortable', label: 'Comfortable', icon: Rows2 },
  { value: 'compact', label: 'Compact', icon: Rows4 },
];
