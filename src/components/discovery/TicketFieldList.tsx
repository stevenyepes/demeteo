import React from 'react';
import { Plus, X } from 'lucide-react';

import { FieldLabel } from '../ui/FieldLabel';

interface TicketFieldListProps {
  label: string;
  values: string[];
  onChange: (next: string[]) => void;
  /** Acceptance criteria are numbered; file paths are not. */
  numbered?: boolean;
  mono?: boolean;
  placeholder?: string;
  addLabel: string;
  /** The `title=` on each row's remove button (§6.6 lists them all). */
  removeTitle: string;
  disabled: boolean;
}

/**
 * A column of removable rows with an `Add` button under it — acceptance
 * criteria and file paths, which are the same control twice
 * (`DISCOVERY_UI_SPEC.md` §5.4).
 *
 * **No drag.** §6.6 is explicit that the attachment dropzone is the only drag
 * surface in this whole feature: acceptance criteria and files are not
 * reorderable by drag, and the numbers beside them are positions in a list
 * rather than handles.
 */
export function TicketFieldList({
  label,
  values,
  onChange,
  numbered = false,
  mono = false,
  placeholder,
  addLabel,
  removeTitle,
  disabled,
}: TicketFieldListProps): React.ReactElement {
  function setAt(index: number, value: string) {
    onChange(values.map((current, i) => (i === index ? value : current)));
  }

  return (
    <div>
      <FieldLabel>{label}</FieldLabel>
      <div className="flex flex-col gap-2">
        {values.map((value, index) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: a criterion has no identity of its own, and keying on its text remounts the input on every keystroke.
            key={index}
            className="flex items-center gap-2"
          >
            {numbered && (
              <span
                aria-hidden="true"
                className="w-[18px] shrink-0 text-right font-mono text-[10px] text-slate-600"
              >
                {index + 1}
              </span>
            )}
            <input
              type="text"
              value={value}
              disabled={disabled}
              aria-label={`${label} ${index + 1}`}
              placeholder={placeholder}
              onChange={(event) => setAt(index, event.target.value)}
              className={`input-field text-[13px] ${mono ? 'font-mono text-xs' : ''}`}
            />
            <button
              type="button"
              title={removeTitle}
              aria-label={removeTitle}
              disabled={disabled}
              onClick={() => onChange(values.filter((_, i) => i !== index))}
              className="shrink-0 rounded-md p-1 text-slate-600 transition hover:bg-ruby-500/[0.08] hover:text-ruby-400 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <X className="h-[15px] w-[15px]" aria-hidden="true" />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        disabled={disabled}
        onClick={() => onChange([...values, ''])}
        className={`mt-2 inline-flex items-center gap-1.5 self-start rounded-md border border-dashed border-white/10 px-3 py-[7px] text-xs text-slate-400 transition hover:border-violet-500/35 hover:bg-violet-500/5 hover:text-violet-300 disabled:cursor-not-allowed disabled:opacity-40 ${
          numbered ? 'ml-[26px]' : ''
        }`}
      >
        <Plus className="h-[13px] w-[13px]" aria-hidden="true" />
        {addLabel}
      </button>
    </div>
  );
}

export default TicketFieldList;
