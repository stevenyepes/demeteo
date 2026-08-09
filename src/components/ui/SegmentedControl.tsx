import type { LucideIcon } from 'lucide-react';
import React from 'react';

import { TONE_CHIP, TONE_TEXT, type RunStatusTone } from '../../lib/runStatus';

export interface SegmentedOption<T extends string | number> {
  value: T;
  label: string;
  icon?: LucideIcon;
  /** Trailing badge, rendered only above zero — the pipeline filter's "needs you 2". */
  count?: number;
  /** Badge tone; defaults to the segment's own selected/idle colour. */
  countTone?: RunStatusTone;
}

export interface SegmentedControlProps<T extends string | number> {
  options: readonly SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /** A radiogroup carries no implicit label, so the group's name is required. */
  ariaLabel: string;
  /** `md` is the run toolbar; `sm` the denser list filter. */
  size?: SegmentedSize;
  className?: string;
  ref?: React.Ref<HTMLDivElement>;
}

export type SegmentedSize = 'sm' | 'md';

/**
 * Exclusive mode/filter picker: one row of segments, exactly one chosen
 * (UI_REDESIGN_PLAN §5.1).
 *
 * It is a `radiogroup`, not a `tablist`: nothing it renders owns a tabpanel, and
 * a tablist promises `aria-controls` and a panel relationship that a filter has
 * no counterpart for. Radiogroup also brings the interaction users already
 * expect from an exclusive choice — one tab stop for the whole group, arrows to
 * move *and* select — which a plain row of buttons costs a Tab press per option
 * to reach.
 *
 * `ref` lands on the group element because at least one caller measures it:
 * `useRunColumnLayout` treats the run view's toggle as chrome and subtracts its
 * height from the graph box, so the ref is load-bearing rather than decoration
 * and must survive an empty `options` array.
 *
 * Values are `string | number` so a segment's identity is a stable key and `===`
 * is the whole selection rule; an object-valued option would need a keying and
 * an equality prop that no caller here wants.
 */
export function SegmentedControl<T extends string | number>({
  options,
  value,
  onChange,
  ariaLabel,
  size = 'md',
  className = '',
  ref,
}: SegmentedControlProps<T>): React.ReactElement {
  const segmentsRef = React.useRef<Array<HTMLButtonElement | null>>([]);

  const selectedIndex = options.findIndex((option) => option.value === value);

  function select(index: number) {
    const option = options[index];
    if (!option || option.value === value) return;
    onChange(option.value);
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (options.length === 0) return;

    const from = selectedIndex >= 0 ? selectedIndex : 0;
    const step = STEP[event.key];
    let next: number;
    if (step !== undefined) {
      next = (from + step + options.length) % options.length;
    } else if (event.key === 'Home') {
      next = 0;
    } else if (event.key === 'End') {
      next = options.length - 1;
    } else {
      return;
    }

    event.preventDefault();
    segmentsRef.current[next]?.focus();
    select(next);
  }

  const density = SIZE[size];

  return (
    <div
      ref={ref}
      role="radiogroup"
      aria-label={ariaLabel}
      data-size={size}
      data-testid="segmented-control"
      onKeyDown={handleKeyDown}
      className={`inline-flex shrink-0 items-center gap-1 rounded-lg border border-white/10 bg-white/[0.02] ${density.group} ${className}`}
    >
      {options.map((option, index) => {
        const selected = index === selectedIndex;
        const Icon = option.icon;
        const hasCount = typeof option.count === 'number' && option.count > 0;

        return (
          <button
            key={option.value}
            ref={(el) => {
              segmentsRef.current[index] = el;
            }}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={hasCount ? `${option.label}, ${option.count}` : undefined}
            tabIndex={selected || (selectedIndex < 0 && index === 0) ? 0 : -1}
            onClick={() => select(index)}
            className={`flex items-center gap-1.5 rounded-md border font-semibold transition focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-cyan-500/50 ${density.segment} ${selected ? TONE_CHIP.cyan : IDLE}`}
          >
            {Icon && <Icon className={density.icon} />}
            {option.label}
            {hasCount && (
              <span
                aria-hidden="true"
                className={`rounded-full bg-white/10 px-1.5 font-mono text-[10px] ${TONE_TEXT[option.countTone ?? (selected ? 'cyan' : 'slate')]}`}
              >
                {option.count}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

const IDLE = 'border-transparent text-slate-400 hover:bg-white/5 hover:text-slate-200';

const STEP: Record<string, number | undefined> = {
  ArrowRight: 1,
  ArrowDown: 1,
  ArrowLeft: -1,
  ArrowUp: -1,
};

const SIZE: Record<SegmentedSize, { group: string; segment: string; icon: string }> = {
  sm: { group: 'p-0.5', segment: 'px-2 py-1 text-[11px]', icon: 'h-3 w-3' },
  md: { group: 'p-1', segment: 'px-3 py-1.5 text-xs', icon: 'h-3.5 w-3.5' },
};

export default SegmentedControl;
