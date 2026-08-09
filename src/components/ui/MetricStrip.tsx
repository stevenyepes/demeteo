/**
 * MetricStrip / Metric — the compact telemetry group that replaces
 * `FeatureDetail/FeatureHeader`'s four stacked stat blocks
 * (UI_REDESIGN_PLAN §5.1). Two decisions live here:
 *
 * **A ticking value must not resize the strip.** Elapsed and cost change every
 * second, and a group whose width follows its digit count drags every wrapped
 * row of the header with it on each tick. So a value reserves the widest string
 * it has been asked to render, measured in `ch`: exact rather than approximate
 * because the value is monospace *and* `tabular-nums`, so every glyph is one
 * advance width. That reserve is a per-instance measurement, which no static
 * Tailwind utility can express — hence the one inline `minWidth`. It resets
 * when `label` changes, so a caller that swaps which metrics it renders in
 * place does not inherit the previous metric's width.
 *
 * **Spacing is `gap` only, with no separator elements.** Cache reads appear
 * solely when non-zero, so `{n > 0 && <Metric … />}` has to cost nothing;
 * dividers between children would render the false branch as a visible gap.
 *
 * Values arrive as display strings — `formatCost` / `formatTokens` /
 * `formatDuration` in `src/lib/utils.ts` own the formatting rules, and a
 * second home for them here would be a second set to keep in sync.
 */

import { useRef } from 'react';
import type { ReactNode } from 'react';

import { TONE_TEXT, type RunStatusTone } from '../../lib/runStatus';

export interface MetricProps {
  label: string;
  value: string;
  /** Status tone for the value; omitted leaves it neutral white. */
  tone?: RunStatusTone;
  tooltip?: string;
  className?: string;
}

export function Metric({
  label,
  value,
  tone,
  tooltip,
  className = '',
}: MetricProps): React.ReactElement {
  const reserve = useRef({ label, chars: 0 });
  if (reserve.current.label !== label) reserve.current = { label, chars: 0 };
  reserve.current.chars = Math.max(reserve.current.chars, value.length);

  return (
    <div
      data-testid="metric"
      data-metric={label}
      title={tooltip}
      className={`flex flex-col justify-center gap-1 shrink-0 ${className}`}
    >
      <span className="text-[10px] font-bold uppercase tracking-wider leading-none text-slate-500">
        {label}
      </span>
      <span
        data-testid="metric-value"
        style={{ minWidth: `${reserve.current.chars}ch` }}
        className={`text-sm font-mono font-bold tabular-nums leading-none whitespace-nowrap ${
          tone ? TONE_TEXT[tone] : 'text-white'
        }`}
      >
        {value}
      </span>
    </div>
  );
}

export interface MetricStripProps {
  children: ReactNode;
  className?: string;
}

export function MetricStrip({ children, className = '' }: MetricStripProps): React.ReactElement {
  return (
    <div
      data-testid="metric-strip"
      className={`glass-panel inline-flex flex-wrap items-center gap-x-5 gap-y-2 min-w-0 px-4 py-2 ${className}`}
    >
      {children}
    </div>
  );
}
