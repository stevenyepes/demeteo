import { Search, X } from 'lucide-react';
import React from 'react';

import { SegmentedControl, type SegmentedOption } from './SegmentedControl';

/** One `<select>` in the bar's trailing group. Sort is one of these rather
 *  than a field of its own: a list that also filters by harness would
 *  otherwise grow a second dropdown mechanism sitting beside the first,
 *  spelled differently. */
export interface ListFilterSelect {
  /** React key and the `aria-label` of the control. */
  label: string;
  value: string;
  options: readonly { value: string; label: string }[];
  onChange: (value: string) => void;
}

export interface ListFilterBarProps<S extends string> {
  segments: readonly SegmentedOption<S>[];
  segment: S;
  onSegmentChange: (segment: S) => void;
  query: string;
  onQueryChange: (query: string) => void;
  selects?: readonly ListFilterSelect[];
  /** Plural, lower case — "pipelines", "discoveries", "threads". Every label
   *  and placeholder in the bar is built from it, so the three tabs read as
   *  one control with one noun swapped rather than three phrasings. */
  noun: string;
  /** Whether a control that *hides* rows is set. Owned by the caller's filter
   *  policy, not re-derived here: this component sees display values, and a
   *  second opinion about what counts as narrowing is the drift. */
  narrowed: boolean;
  onClear: () => void;
  /** The unfiltered length, which is what tells "your filter matched nothing"
   *  apart from "this project has none" — the second is not a filter outcome
   *  and a reset would not help it, so the bar stays silent and the caller
   *  renders its own empty state. */
  total: number;
  resultCount: number;
  className?: string;
  testId?: string;
}

/**
 * Segment + query + selects for a project-list tab (UI_REDESIGN_PLAN §3.2,
 * §5.2), shared by Pipelines, Discovery and Ask.
 *
 * It holds the *chrome* and nothing else: which rows survive is each list's
 * own policy (`lib/pipelineFilter.ts`, `lib/sessionFilter.ts`), which is where
 * it is answerable from a test with plain objects. The three tabs share this
 * file so a control added to one arrives in the others by construction — they
 * were separately spelled bars, which is how a user got a segment row in one
 * tab, a bare search box in the next, and no way to tell whether the missing
 * controls were a decision or an omission.
 *
 * Three choices not recoverable from the markup, inherited from the pipeline
 * bar this generalises:
 *
 * **Ordering controls are `<select>`s, not a second `SegmentedControl`.** Two
 * radiogroups in one bar both paint their selection in `TONE_CHIP.cyan`, so
 * the segment row — which carries the needs-you promise and is scanned on
 * every visit — would compete with a control most users set once. The native
 * control also collapses several options into one line of chrome and arrives
 * keyboard- and AT-complete.
 *
 * **The bar owns "your filter matched nothing"; the caller owns "this project
 * has none".** The first is a state the user caused, so the undo belongs
 * beside the controls that caused it.
 *
 * **No debounce.** Both filter policies return their input array's identity
 * when nothing changed, which is what keeps a memoized list cheap per
 * keystroke; a debounce here would buy nothing and cost the input its
 * immediacy. If a profile ever says otherwise, that measurement is the
 * argument, not this.
 */
export function ListFilterBar<S extends string>({
  segments,
  segment,
  onSegmentChange,
  query,
  onQueryChange,
  selects = [],
  noun,
  narrowed,
  onClear,
  total,
  resultCount,
  className = '',
  testId = 'list-filter-bar',
}: ListFilterBarProps<S>): React.ReactElement {
  return (
    <div className={`flex flex-col gap-2 ${className}`} data-testid={testId}>
      <div className="flex flex-wrap items-center gap-2">
        <SegmentedControl
          options={segments}
          value={segment}
          onChange={onSegmentChange}
          ariaLabel={`Filter ${noun}`}
          size="sm"
        />

        <div className="relative min-w-0 flex-1 sm:max-w-xs">
          {/* Escape is not bound to "clear": `useKeyboardShortcuts` owns it
              globally and unguarded, so a second meaning here would fire the
              app-level handler on the same press. */}
          <Search
            className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-slate-500"
            aria-hidden
          />
          <input
            type="search"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder={`Filter ${noun}...`}
            aria-label={`Filter ${noun} by text`}
            className="w-full rounded-md border border-white/5 bg-black/30 py-1.5 pl-7 pr-7 text-[11px] text-white placeholder-slate-600 focus:border-cyan-500/30 focus:outline-none"
          />
          {query !== '' && (
            <button
              type="button"
              onClick={() => onQueryChange('')}
              aria-label="Clear the filter text"
              className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
            >
              <X className="h-3 w-3" />
            </button>
          )}
        </div>

        {selects.map((select) => (
          <select
            key={select.label}
            value={select.value}
            onChange={(event) => select.onChange(event.target.value)}
            aria-label={select.label}
            className="shrink-0 rounded-md border border-white/10 bg-black/20 px-2 py-1.5 text-[11px] font-mono text-slate-300 outline-none hover:border-white/20 focus:border-cyan-500/50"
          >
            {select.options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        ))}
      </div>

      {total > 0 && resultCount === 0 && (
        <p role="status" className="flex items-center gap-2 text-[11px] text-slate-500">
          No {noun} match this filter.
          {narrowed && (
            <button
              type="button"
              onClick={onClear}
              className="font-medium text-cyan-400 transition-colors hover:text-cyan-300"
            >
              Clear filters
            </button>
          )}
        </p>
      )}
    </div>
  );
}

export default ListFilterBar;
