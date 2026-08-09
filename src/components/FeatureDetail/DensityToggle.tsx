import { Rows2, Rows4 } from 'lucide-react';

import type { Density } from '../../lib/density';
import { SegmentedControl, type SegmentedOption } from '../ui/SegmentedControl';

export interface DensityToggleProps {
  value: Density;
  onChange: (value: Density) => void;
  className?: string;
}

/**
 * Timeline density picker (`docs/UI_REDESIGN_PLAN.md` §3.7).
 *
 * Value and setter are props: the choice is persisted in Phase 6, through
 * `get_app_session`/`set_app_session` rather than component state, so owning it
 * here would be a second home to unpick.
 */
export function DensityToggle({
  value,
  onChange,
  className = '',
}: DensityToggleProps): React.ReactElement {
  return (
    <SegmentedControl
      options={OPTIONS}
      value={value}
      onChange={onChange}
      ariaLabel="Timeline density"
      size="sm"
      className={className}
    />
  );
}

const OPTIONS: readonly SegmentedOption<Density>[] = [
  { value: 'comfortable', label: 'Comfortable', icon: Rows2 },
  { value: 'compact', label: 'Compact', icon: Rows4 },
];
