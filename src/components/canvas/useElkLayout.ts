/**
 * Elk auto-layout for the workflow canvas, run off the main thread.
 *
 * elkjs is CPU-heavy for the layered algorithm; running it in a Web Worker
 * keeps the battery-sensitive Tauri webview responsive (PRD §6.1, "elk in a
 * worker"). We use `elk-api` with elkjs's own worker script rather than
 * wrapping `elk.bundled.js` in a hand-rolled worker: the bundled build sniffs
 * its environment (`elk.bundled.js` — no `document` + a `self` global means
 * "I am the worker script") and skips exporting its `Worker` shim, so
 * importing it inside a real worker crashes with "undefined is not a
 * constructor (evaluating 'new _Worker(url)')" in WKWebView.
 *
 * The worker is created on first use (never at import time) so the fixture
 * render tests — which don't press the layout button — never spawn one under
 * jsdom.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import ELK, { type ELK as Elk, type ElkNode } from 'elkjs/lib/elk-api.js';
import ElkWorker from 'elkjs/lib/elk-worker.min.js?worker';

/** Default card dimensions when React Flow hasn't measured a node yet. */
const DEFAULT_NODE = { width: 240, height: 64 };

/**
 * Everything except the direction, which the caller picks from the space the
 * canvas actually has (`layoutDirection.ts`) — top-to-bottom matches the
 * migration's vertical column, left-to-right uses the width of a landscape
 * window. Spacing here is mirrored by the direction estimate; keep them in
 * step.
 */
const LAYOUT_OPTIONS: Record<string, string> = {
  'elk.algorithm': 'layered',
  'elk.layered.spacing.nodeNodeBetweenLayers': '64',
  'elk.spacing.nodeNode': '48',
  'elk.layered.nodePlacement.strategy': 'NETWORK_SIMPLEX',
};

export type ElkLayoutResult = { id: string; x: number; y: number }[];

export interface LayoutNode {
  id: string;
  /** measured dimensions, if React Flow has them yet */
  measured?: { width?: number; height?: number };
}

export interface LayoutEdge {
  id: string;
  source: string;
  target: string;
}

export function useElkLayout() {
  const elkRef = useRef<Elk | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(
    () => () => {
      elkRef.current?.terminateWorker();
      elkRef.current = null;
    },
    [],
  );

  const layout = useCallback(
    async (
      nodes: LayoutNode[],
      edges: LayoutEdge[],
      direction: 'DOWN' | 'RIGHT' = 'DOWN',
    ): Promise<ElkLayoutResult> => {
      if (!elkRef.current) {
        elkRef.current = new ELK({
          workerFactory: () => new ElkWorker(),
        });
      }

      const graph: ElkNode = {
        id: 'root',
        layoutOptions: { ...LAYOUT_OPTIONS, 'elk.direction': direction },
        children: nodes.map((n) => ({
          id: n.id,
          width: n.measured?.width ?? DEFAULT_NODE.width,
          height: n.measured?.height ?? DEFAULT_NODE.height,
        })),
        edges: edges.map((e) => ({
          id: e.id,
          sources: [e.source],
          targets: [e.target],
        })),
      };

      setRunning(true);
      try {
        const laid = await elkRef.current.layout(graph);
        return (laid.children ?? []).map((c) => ({
          id: c.id,
          x: c.x ?? 0,
          y: c.y ?? 0,
        }));
      } catch {
        // A layout failure must never wedge the canvas — return no moves and
        // the caller keeps the persisted positions.
        return [];
      } finally {
        setRunning(false);
      }
    },
    [],
  );

  return { layout, running };
}
