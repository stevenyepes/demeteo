/**
 * Measure the run column and the chrome above its graph, and turn those two
 * numbers into the layout the run view renders with.
 *
 * This is the *measuring* half of the run layout, deliberately split from the
 * verdicts it feeds: `pickRunLayout` (`runLayout.ts`) and `planLayout` /
 * `graphContainer` / `graphBoxHeight` (`canvas/layoutDirection.ts`) read no DOM
 * and are answerable from a test with a few numbers. Everything that needs a
 * live element is here, so `FeatureDetail` consumes one hook instead of
 * carrying four pieces of state and two observers of its own.
 */
import { useEffect, useMemo, useState } from 'react';
import { graphBoxHeight, graphContainer, MAX_ZOOM, planLayout } from './canvas/layoutDirection';
import type { LayoutPlan } from './canvas/layoutDirection';
import type { WorkflowDefinitionV2 } from './canvas/types';
import { pickRunLayout, type RunColumnSize, type RunLayoutMode } from './runLayout';

export interface RunColumnLayout {
  /** `ref` for the run column itself. */
  setRunColumnEl: (el: HTMLDivElement | null) => void;
  /** `ref` for the meta track — pass `undefined` when it sits *beside* the
   *  graph rather than above it, since then it is not the graph's chrome. */
  setMetaChromeEl: (el: HTMLDivElement | null) => void;
  /** `ref` for the Graph|Timeline toggle, which always stacks above the graph. */
  setToggleChromeEl: (el: HTMLDivElement | null) => void;
  runLayout: RunLayoutMode;
  graphPlan: LayoutPlan;
  /** Height, in px, for the graph box element. */
  graphBoxPx: number;
}

export function useRunColumnLayout(graphDef: WorkflowDefinitionV2 | null): RunColumnLayout {
  /** The run column measures *itself* — a media query or `window.innerWidth`
   *  is blind to the side rail and to a half-width window. Same discipline as
   *  `WorkflowCanvas`: 8px rounding plus a `prev`-returning guard, so a resize
   *  drag can't push a fresh size per frame into anything that feeds elk. Held
   *  in state, not a ref: the column mounts only once `loading` clears, after
   *  a `[]` effect would have run. jsdom has no `ResizeObserver` — hence the
   *  guard, and why every verdict below is unit-tested in its own module. */
  const [runColumnEl, setRunColumnEl] = useState<HTMLDivElement | null>(null);
  const [runColumnSize, setRunColumnSize] = useState<RunColumnSize | null>(null);
  useEffect(() => {
    if (!runColumnEl || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => {
      const box = entry.contentRect;
      const next = { width: Math.round(box.width / 8) * 8, height: Math.round(box.height / 8) * 8 };
      setRunColumnSize((prev) => (prev && prev.width === next.width && prev.height === next.height ? prev : next));
    });
    observer.observe(runColumnEl);
    return () => observer.disconnect();
  }, [runColumnEl]);
  /** The chrome stacked above the graph inside that column is measured the same
   *  way, because the box's own space is the column minus it. Two elements, one
   *  observer: the meta track — registered only when it stacks above the graph
   *  rather than beside it — and the view toggle. */
  const [metaChromeEl, setMetaChromeEl] = useState<HTMLDivElement | null>(null);
  const [toggleChromeEl, setToggleChromeEl] = useState<HTMLDivElement | null>(null);
  const [chromeHeight, setChromeHeight] = useState(0);
  useEffect(() => {
    const above = [metaChromeEl, toggleChromeEl].filter((el): el is HTMLDivElement => el !== null);
    if (above.length === 0 || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      const next = Math.round(above.reduce((sum, el) => sum + el.offsetHeight, 0) / 8) * 8;
      setChromeHeight((prev) => (prev === next ? prev : next));
    });
    above.forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  }, [metaChromeEl, toggleChromeEl]);
  /** Both verdicts belong to pure modules; this hook only measures and
   *  consumes. `graphContainer` is what makes the plan honest: sized against the
   *  whole column, a 4K window answers `DOWN` for a height the box hasn't got.
   *  Rounded and guarded, so no observer tick reaches elk. */
  const runLayout = pickRunLayout(runColumnSize);
  const graphBox = useMemo(() => graphContainer(runColumnSize, chromeHeight), [runColumnSize, chromeHeight]);
  const graphPlan = useMemo(() => {
    const nodes = graphDef?.nodes.map((n) => ({ id: n.id })) ?? [];
    const edges = graphDef?.edges.map((e) => ({ id: `${e.from}>${e.to}`, source: e.from, target: e.to })) ?? [];
    return planLayout(nodes, edges, graphBox, MAX_ZOOM);
  }, [graphDef, graphBox]);
  const graphBoxPx = graphBoxHeight(graphPlan, graphBox?.height ?? 0);

  return {
    setRunColumnEl,
    setMetaChromeEl,
    setToggleChromeEl,
    runLayout,
    graphPlan,
    graphBoxPx,
  };
}
