import React from 'react';
import { ChevronDown } from 'lucide-react';

import { FieldLabel } from '../ui/FieldLabel';

interface LabelledSelectProps {
  label: string;
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
  options: { value: string; label: string }[];
}

/**
 * One `.selwrap` of `DISCOVERY_UI_SPEC.md` §5.6: a native select carrying the
 * chevron the platform control does not draw once `appearance:none` removes
 * its own.
 *
 * **Unset is a real option, not a placeholder.** A ticket that names no model
 * runs on the project's own default, which §5.4 makes a choice the user is
 * allowed to make — so the empty entry stays selectable rather than being the
 * disabled prompt a form usually puts there.
 */
export function LabelledSelect({
  label,
  value,
  disabled,
  onChange,
  options,
}: LabelledSelectProps): React.ReactElement {
  const id = `ticket-${label.toLowerCase()}`;
  return (
    <div>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <div className="relative">
        <select
          id={id}
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          className="input-field cursor-pointer appearance-none bg-[var(--bg-app)] pr-[30px] text-[13px]"
        >
          <option value="">—</option>
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        <ChevronDown
          aria-hidden="true"
          className="pointer-events-none absolute right-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-600"
        />
      </div>
    </div>
  );
}

export default LabelledSelect;
