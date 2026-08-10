import React, { useId } from 'react';
import { X } from 'lucide-react';

import { TabBar, type TabDef } from './TabBar';

export interface InspectorProps<T extends string> {
  title: React.ReactNode;
  /** Leading glyph beside the title; the caller sizes and tones it. */
  icon?: React.ReactNode;
  /** Chip row under the title — whatever identifies the subject at a glance. */
  meta?: React.ReactNode;
  /** Omitted = no dismiss affordance (a docked, always-present inspector). */
  onDismiss?: () => void;
  dismissLabel?: string;
  tabs: readonly TabDef<T>[];
  activeTab: T;
  onTabChange: (tab: T) => void;
  /** A tablist carries no implicit name. */
  ariaLabel: string;
  /** Body of the selected tab. The inspector never scrolls it — a tab that
   *  needs scrolling brings its own container, so one tab can hold a fixed
   *  header over a scrolling log and another a single scrolling column. */
  children: React.ReactNode;
  className?: string;
}

/**
 * Header + tab strip + body shell for a detail pane (UI_REDESIGN_PLAN §5.1).
 *
 * Generic over the tab key so a caller's own union survives the round trip:
 * `onTabChange` hands back `T`, and a key outside the union cannot be spelled.
 * Nothing here knows what is being inspected — header content, tab list and the
 * selected tab's body are all the caller's.
 *
 * The strip is `TabBar` at its dense size, not a second one: this shell owns
 * the ids on both ends of the `aria-controls`/`aria-labelledby` pair and hands
 * them down, which is the only thing the strip could not derive alone.
 *
 * One panel element stays mounted across tabs and the caller swaps its
 * children, so every tab's `aria-controls` resolves. A panel per tab with all
 * but one unmounted — the shape the conditional bodies suggest — would leave
 * the idle tabs pointing at ids that are not in the document.
 */
/** The pane's own chrome, exported so a caller that has *nothing* to inspect
 *  can render the same surface without a tab strip — an always-present pane
 *  whose empty state looked like a different component would read as broken
 *  rather than as empty. */
/**
 * The inspector is a card, not a sidebar.
 *
 * It read as one for as long as it was docked against an edge — a left border,
 * no radius, a sidebar fill and a heavier `backdrop-blur-xl` than any panel in
 * the app. Seated as the middle of the run's three tracks, that made the one
 * surface between two cards the only thing on screen with square corners and
 * a border on a single side. `glass-panel` brings all four decisions back to
 * the card language, and the blur down from 24px to the panel's 12 — the run
 * column already stacks translucency and the plan's §7 treats it as a budget.
 */
export const INSPECTOR_SURFACE = 'glass-panel flex h-full flex-col overflow-hidden';

export function Inspector<T extends string>({
  title,
  icon,
  meta,
  onDismiss,
  dismissLabel = 'Close panel',
  tabs,
  activeTab,
  onTabChange,
  ariaLabel,
  children,
  className = '',
}: InspectorProps<T>): React.ReactElement {
  const baseId = useId();
  const panelId = `${baseId}-panel`;
  const tabDomId = (value: T) => `${baseId}-tab-${value}`;

  const hasActiveTab = tabs.some((tab) => tab.value === activeTab);

  return (
    <div
      data-testid="inspector"
      className={`${INSPECTOR_SURFACE} ${className}`}
    >
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-white/5 px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            {icon}
            <h3 className="truncate font-heading text-sm font-bold uppercase tracking-wider text-white">
              {title}
            </h3>
          </div>
          {meta && <div className="mt-1.5 flex flex-wrap items-center gap-2">{meta}</div>}
        </div>
        {onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            className="shrink-0 rounded-lg bg-white/5 p-1.5 text-slate-400 transition hover:bg-white/10 hover:text-white"
            title={dismissLabel}
            aria-label={dismissLabel}
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </div>

      <TabBar
        size="sm"
        className="shrink-0"
        tabs={tabs}
        activeTab={activeTab}
        onChange={onTabChange}
        ariaLabel={ariaLabel}
        panelId={panelId}
        tabDomId={tabDomId}
      />

      <div
        id={panelId}
        role="tabpanel"
        aria-labelledby={hasActiveTab ? tabDomId(activeTab) : undefined}
        className="min-h-0 flex-1 overflow-hidden"
      >
        {children}
      </div>
    </div>
  );
}

export default Inspector;
