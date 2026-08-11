import React, { useRef } from 'react';

import { TONE_TEXT } from '../../lib/runStatus';
import { nextIndexForKey } from './rovingIndex';

export interface TabDef<T extends string = string> {
  value: T;
  label: string;
  icon?: React.ReactNode;
}

/** `md` is a page-level section switcher; `sm` the denser strip inside a pane. */
export type TabBarSize = 'sm' | 'md';

export interface TabBarProps<T extends string> {
  tabs: readonly TabDef<T>[];
  activeTab: T;
  onChange: (value: T) => void;
  /** A tablist carries no implicit name. */
  ariaLabel: string;
  /**
   * Id of the `tabpanel` the tabs control. Omitted when the caller renders the
   * tab bodies without one: `aria-controls` is then left off entirely rather
   * than pointed at an id that is not in the document.
   */
  panelId?: string;
  /** DOM id per tab, so a panel can name its tab through `aria-labelledby`. */
  tabDomId?: (value: T) => string;
  size?: TabBarSize;
  className?: string;
}

/**
 * The one tab strip (UI_REDESIGN_PLAN §5.1, audit F28/F36).
 *
 * It is a `tablist` where `SegmentedControl` is a `radiogroup`: the distinction
 * that file records is whether the row switches a body of content or filters
 * one. Arrow keys move *and* select, matching it, so the two exclusive-choice
 * rows in the same app do not disagree about what an arrow key means.
 *
 * `panelId`/`tabDomId` come from the caller because only it knows whether the
 * bodies it swaps are a real `tabpanel`. `Inspector` owns both ends of that
 * pair; the settings screens swap plain sections and pass neither, so the
 * strip emits no `aria-controls` rather than one pointing nowhere.
 *
 * `size` exists because the settings screens and a docked inspector want the
 * same strip at two densities. It is the only axis they are allowed to differ
 * on — the selected treatment is one decision, taken here, so the two cannot
 * drift back into the two implementations this replaced.
 */
export function TabBar<T extends string>({
  tabs,
  activeTab,
  onChange,
  ariaLabel,
  panelId,
  tabDomId,
  size = 'md',
  className = '',
}: TabBarProps<T>): React.ReactElement {
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const activeIndex = tabs.findIndex((tab) => tab.value === activeTab);
  const density = SIZE[size];

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    const next = nextIndexForKey(event.key, activeIndex, tabs.length);
    if (next === null) return;

    event.preventDefault();
    tabRefs.current[next]?.focus();
    const target = tabs[next];
    if (target && target.value !== activeTab) onChange(target.value);
  }

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      data-size={size}
      onKeyDown={handleKeyDown}
      className={`flex gap-1 border-b border-white/5 ${density.strip} ${className}`}
    >
      {tabs.map((tab, index) => {
        const selected = index === activeIndex;
        return (
          <button
            key={tab.value}
            ref={(el) => {
              tabRefs.current[index] = el;
            }}
            type="button"
            role="tab"
            id={tabDomId?.(tab.value)}
            aria-selected={selected}
            aria-controls={panelId}
            tabIndex={selected || (activeIndex < 0 && index === 0) ? 0 : -1}
            onClick={() => onChange(tab.value)}
            className={`flex items-center gap-2 border-b-2 transition-all ${density.tab} ${
              selected ? SELECTED : IDLE
            }`}
          >
            {tab.icon}
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}

const SELECTED = `border-cyan-500 ${TONE_TEXT.cyan}`;
const IDLE = 'border-transparent text-slate-400 hover:text-slate-200';

const SIZE: Record<TabBarSize, { strip: string; tab: string }> = {
  sm: { strip: 'px-3 pt-2', tab: 'rounded-t-md px-3 py-1.5 text-xs font-semibold' },
  md: { strip: 'pb-px', tab: 'px-4 py-2.5 text-sm font-heading font-medium' },
};

export default TabBar;
