/**
 * `WorkflowCanvas` — the single graph surface the DAG-workflows work is built
 * around (PRD §6.1). This is the **read-only** foundation (task P2.1): it
 * renders a migrated schema-v2 definition as a React Flow graph with the app's
 * dark-neon look, a minimap that auto-hides on small graphs, fit-view, an elk
 * auto-layout button (worker-backed), and keyboard navigation between nodes.
 *
 * Later phases layer on top without forking this component: the live status
 * overlay + Graph|Timeline toggle (P2.2), the node drill-down panel (P2.3),
 * and design-mode editing (Phase 3). `onNodeActivate` is the seam the panel
 * plugs into.
 */
import { useCallback, useEffect, useMemo, useRef } from 'react';
import {
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  Panel,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type NodeMouseHandler,
} from '@xyflow/react';
import { LayoutGrid } from 'lucide-react';
import '@xyflow/react/dist/style.css';

import { WorkflowNode } from './nodes/WorkflowNode';
import { toFlowGraph, type WorkflowFlowNode } from './flowGraph';
import { useElkLayout } from './useElkLayout';
import type { NodeRunStatus, WorkflowDefinitionV2 } from './types';

/** Below this node count the minimap is noise, so it auto-hides (PRD §6.1). */
const MINIMAP_THRESHOLD = 8;

const NODE_TYPES = { workflow: WorkflowNode };

export interface WorkflowCanvasProps {
  definition: WorkflowDefinitionV2;
  /** node id → live run state for the run-mode overlay (P2.2). */
  statusByNode?: Record<string, NodeRunStatus>;
  /** Fired on click or Enter over a node — the panel-open seam (P2.3). */
  onNodeActivate?: (nodeId: string) => void;
  className?: string;
}

function CanvasInner({
  definition,
  statusByNode,
  onNodeActivate,
  className = '',
}: WorkflowCanvasProps) {
  const base = useMemo(
    () => toFlowGraph(definition, { statusByNode }),
    [definition, statusByNode],
  );

  const [nodes, setNodes, onNodesChange] = useNodesState(base.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(base.edges);
  const { fitView, getNodes } = useReactFlow();
  const { layout, running } = useElkLayout();
  const wrapperRef = useRef<HTMLDivElement>(null);

  // Re-seed when the definition (or its overlay) changes identity. Positions
  // come from the definition, so this also discards any prior elk layout —
  // intended: a new definition owns its own layout.
  useEffect(() => {
    setNodes(base.nodes);
    setEdges(base.edges);
  }, [base, setNodes, setEdges]);

  const showMiniMap = nodes.length >= MINIMAP_THRESHOLD;

  const runAutoLayout = useCallback(async () => {
    const current = getNodes() as WorkflowFlowNode[];
    const positions = await layout(
      current.map((n) => ({ id: n.id, measured: n.measured })),
      edges.map((e) => ({ id: e.id, source: e.source, target: e.target })),
    );
    if (positions.length === 0) return;
    const byId = new Map(positions.map((p) => [p.id, p]));
    setNodes((prev) =>
      prev.map((n) => {
        const p = byId.get(n.id);
        return p ? { ...n, position: { x: p.x, y: p.y } } : n;
      }),
    );
    // Let React Flow commit the new positions before fitting the viewport.
    window.requestAnimationFrame(() => void fitView({ duration: 300 }));
  }, [edges, getNodes, layout, setNodes, fitView]);

  const activate = useCallback(
    (id: string) => {
      setNodes((prev) => prev.map((n) => ({ ...n, selected: n.id === id })));
      onNodeActivate?.(id);
    },
    [setNodes, onNodeActivate],
  );

  const onNodeClick: NodeMouseHandler = useCallback(
    (_evt, node) => activate(node.id),
    [activate],
  );

  // Arrow keys move the selection to the nearest node in that direction;
  // Enter activates it. Built on node positions so it works for both the
  // migrated vertical chains and future fanned-out layouts.
  const onKeyDown = useCallback(
    (evt: React.KeyboardEvent<HTMLDivElement>) => {
      const current = getNodes() as WorkflowFlowNode[];
      if (current.length === 0) return;
      const selected = current.find((n) => n.selected) ?? null;

      if (evt.key === 'Enter') {
        if (selected) {
          evt.preventDefault();
          onNodeActivate?.(selected.id);
        }
        return;
      }

      const dirs: Record<string, (dx: number, dy: number) => boolean> = {
        ArrowDown: (_dx, dy) => dy > 0,
        ArrowUp: (_dx, dy) => dy < 0,
        ArrowRight: (dx) => dx > 0,
        ArrowLeft: (dx) => dx < 0,
      };
      const keep = dirs[evt.key];
      if (!keep) return;
      evt.preventDefault();

      if (!selected) {
        activate(current[0].id);
        return;
      }
      const from = selected.position;
      let best: { id: string; d: number } | null = null;
      for (const n of current) {
        if (n.id === selected.id) continue;
        const dx = n.position.x - from.x;
        const dy = n.position.y - from.y;
        if (!keep(dx, dy)) continue;
        const d = dx * dx + dy * dy;
        if (!best || d < best.d) best = { id: n.id, d };
      }
      if (best) activate(best.id);
    },
    [getNodes, activate, onNodeActivate],
  );

  return (
    <div
      ref={wrapperRef}
      className={`relative h-full w-full outline-none ${className}`}
      tabIndex={0}
      onKeyDown={onKeyDown}
      role="application"
      aria-label={`Workflow graph: ${definition.name}`}
      data-testid="workflow-canvas"
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        nodeTypes={NODE_TYPES}
        // Read-only foundation: no connecting/dragging edges, but nodes stay
        // selectable so click + keyboard navigation land somewhere.
        nodesConnectable={false}
        nodesDraggable={false}
        elementsSelectable
        fitView
        proOptions={{ hideAttribution: true }}
        minZoom={0.2}
        maxZoom={1.75}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="#334155" />
        <Controls showInteractive={false} />
        {showMiniMap && (
          <MiniMap
            pannable
            zoomable
            nodeColor="#1e293b"
            maskColor="rgba(2,6,23,0.6)"
            className="!bg-slate-900/80 !border !border-slate-700/60"
          />
        )}
        <Panel position="top-right">
          <button
            type="button"
            onClick={runAutoLayout}
            disabled={running}
            className="flex items-center gap-1.5 rounded-lg border border-slate-700/60 bg-slate-900/80 px-2.5 py-1.5 text-xs font-medium text-slate-200 backdrop-blur-sm transition-colors hover:border-slate-600 hover:text-white disabled:opacity-50"
            title="Auto-layout"
          >
            <LayoutGrid className="h-3.5 w-3.5" />
            {running ? 'Laying out…' : 'Auto-layout'}
          </button>
        </Panel>
      </ReactFlow>
    </div>
  );
}

export function WorkflowCanvas(props: WorkflowCanvasProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  );
}
