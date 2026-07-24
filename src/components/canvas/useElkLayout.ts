/**
 * Client side of the elk auto-layout worker (`lib/elkLayout.worker.ts`). Owns a
 * single lazily-spawned worker for the canvas and exposes a `layout()` that
 * resolves node positions off the main thread. The worker is created on first
 * use (never at import time) so the fixture render tests — which don't press
 * the layout button — never spawn one under jsdom.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import type {
  ElkLayoutRequest,
  ElkLayoutResult,
} from '../../lib/elkLayout.worker';

/** Default card dimensions when React Flow hasn't measured a node yet. */
const DEFAULT_NODE = { width: 240, height: 64 };

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
  const workerRef = useRef<Worker | null>(null);
  const [running, setRunning] = useState(false);

  useEffect(
    () => () => {
      workerRef.current?.terminate();
      workerRef.current = null;
    },
    [],
  );

  const layout = useCallback(
    (nodes: LayoutNode[], edges: LayoutEdge[]): Promise<ElkLayoutResult> => {
      if (!workerRef.current) {
        workerRef.current = new Worker(
          new URL('../../lib/elkLayout.worker.ts', import.meta.url),
          { type: 'module' },
        );
      }
      const worker = workerRef.current;

      const request: ElkLayoutRequest = {
        nodes: nodes.map((n) => ({
          id: n.id,
          width: n.measured?.width ?? DEFAULT_NODE.width,
          height: n.measured?.height ?? DEFAULT_NODE.height,
        })),
        edges: edges.map((e) => ({ id: e.id, source: e.source, target: e.target })),
      };

      setRunning(true);
      return new Promise<ElkLayoutResult>((resolve) => {
        const onMessage = (event: MessageEvent<ElkLayoutResult>) => {
          worker.removeEventListener('message', onMessage);
          setRunning(false);
          resolve(event.data);
        };
        worker.addEventListener('message', onMessage);
        worker.postMessage(request);
      });
    },
    [],
  );

  return { layout, running };
}
