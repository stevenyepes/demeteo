import type { ReactNode } from 'react';

import { TabBar, type TabDef } from '../ui/TabBar';

export type InspectorPane = 'step' | 'sync';

interface InspectorColumnProps {
  pane: InspectorPane;
  onPaneChange: (pane: InspectorPane) => void;
  /** Rendered when `pane === 'step'`; the existing step inspector, unchanged. */
  stepInspector: ReactNode;
  syncPanel: ReactNode;
  /** > 0 puts a count on the Sync tab. */
  syncBadge: number;
  className?: string;
}

/**
 * The inspector column's own pane switch — one level above the step
 * inspector's tabs, and the reason the Sync pane is not a fifth one of those.
 *
 * `NodePanel` owns the only tab strip in this column, it is scoped to a node,
 * and the canvas mounts it too — so it may not learn feature-scoped state, and
 * a tab sitting beside Overview / Live / Output / Actions would claim to
 * describe the selected step. `ActivityPanel` records the same mistake being
 * reverted once already: a run-level feed sat under a node-scoped tab until
 * Phase 5 moved it out. A pane that *replaces* the step pane claims nothing
 * about a step.
 *
 * **Exactly one pane is mounted at a time**, which is not a rendering
 * preference: `Inspector` stamps `data-testid="inspector"` and eight
 * assertions across four suites reach for it by that id, which throws on a
 * second match. Keeping both mounted and hiding one with CSS would break them
 * all, invisibly to tsc.
 *
 * `panelId`/`tabDomId` are omitted on purpose. The two bodies are two separate
 * cards and the step pane's own `Inspector` already owns a real `tabpanel`, so
 * an `aria-controls` from here would point at an id that is absent from the
 * document half the time — `TabBar` leaves the attribute off entirely rather
 * than emit that.
 *
 * The strip also moves where `Enter` lands: `focusInspectorPane` takes the
 * first `[role="tab"][tabindex="0"]` inside the wrapper, which is now Step/Sync
 * rather than Overview. That is the outermost choice in the column and the
 * right first stop, but it is a behaviour change, not a side effect.
 */
export function InspectorColumn({
  pane,
  onPaneChange,
  stepInspector,
  syncPanel,
  syncBadge,
  className = '',
}: InspectorColumnProps) {
  const tabs: readonly TabDef<InspectorPane>[] = [
    { value: 'step', label: 'Step' },
    { value: 'sync', label: syncBadge > 0 ? `Sync · ${syncBadge}` : 'Sync' },
  ];

  return (
    <div
      data-testid="inspector-column"
      data-pane={pane}
      className={`flex h-full min-h-0 flex-col ${className}`}
    >
      <TabBar
        size="sm"
        className="shrink-0"
        tabs={tabs}
        activeTab={pane}
        onChange={onPaneChange}
        ariaLabel="Inspector pane"
      />
      <div key={pane} className="mt-2 min-h-0 flex-1 animate-fade-in">
        {pane === 'step' ? stepInspector : syncPanel}
      </div>
    </div>
  );
}

export default InspectorColumn;
