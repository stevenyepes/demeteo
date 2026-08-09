import type { ReactNode } from 'react';

import type { InspectorLayoutMode } from '../runLayout';
import { SplitPane } from '../ui/SplitPane';

interface RunPanesProps {
  layout: InspectorLayoutMode;
  /** The run surface's own height in px, for the cases that state one: a graph
   *  box, or a `'side'` row where both panes have to agree on one. `null` lets
   *  the surface flow and the run column carry the scroll. */
  surfaceHeightPx: number | null;
  runSurface: ReactNode;
  inspector: ReactNode;
  inspectorWidth: number;
  onInspectorWidthCommit: (width: number) => void;
}

/**
 * The run surface and the step inspector, placed (UI_REDESIGN_PLAN §3.1, §4.1).
 *
 * Side by side, the pair owns a fixed box and the run surface scrolls *inside*
 * it, so the inspector holds a stable reading position while a 30-step run
 * moves past it. Stacked, the run keeps the column's own scroll and the
 * inspector drops below with a height of its own — the tabs inside it size
 * against their container rather than their content, so an auto-height box
 * would collapse them.
 *
 * Either way the inspector is rendered. It has no closed state to fall into
 * (§7, settled 2026-08-08); a narrow column moves it, and nothing hides it.
 *
 * `data-run-scroll` marks whichever element the run surface actually scrolls in,
 * which is this one only in the side layout — stacked, the run column upstream
 * carries it. `useHeaderCollapse` reads the attribute rather than the layout.
 */
export function RunPanes({
  layout,
  surfaceHeightPx,
  runSurface,
  inspector,
  inspectorWidth,
  onInspectorWidthCommit,
}: RunPanesProps) {
  if (layout === 'side') {
    return (
      <div
        className="w-full shrink-0"
        style={surfaceHeightPx === null ? undefined : { height: surfaceHeightPx }}
      >
        <SplitPane
          label="Resize step inspector"
          primary={
            <div data-run-scroll className="h-full overflow-y-auto overflow-x-hidden pr-4">
              {runSurface}
            </div>
          }
          secondary={inspector}
          secondaryWidth={inspectorWidth}
          onSecondaryWidthCommit={onInspectorWidthCommit}
        />
      </div>
    );
  }

  return (
    <>
      {surfaceHeightPx === null ? (
        runSurface
      ) : (
        <div className="w-full shrink-0" style={{ height: surfaceHeightPx }}>
          {runSurface}
        </div>
      )}
      <div className="mt-6 h-[28rem] w-full shrink-0">{inspector}</div>
    </>
  );
}

export default RunPanes;
