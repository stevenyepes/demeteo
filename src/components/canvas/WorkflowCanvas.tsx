/**
 * `WorkflowCanvas` — the single graph surface the DAG-workflows work is built
 * around (PRD §6.1), in both of its modes.
 *
 * **Run mode** (the default, P2.1–P2.6): a migrated schema-v2 definition
 * rendered read-only with the live status overlay, minimap, fit-view, elk
 * auto-layout, and keyboard navigation. `onNodeActivate` is the drill-down
 * panel's seam.
 *
 * **Design mode** (`mode="design"`, task P3.1): the same graph, editable.
 * Nodes drag from the `Palette`, edges connect between handles, and every
 * gesture is validated *before* it lands by `connectRules.ts` — the
 * client-side mirror of the Rust structural lint, so the editor refuses
 * exactly the shapes the engine refuses (cycles, an edge out of `finalize`,
 * incompatible ports). Dragging from an output handle onto empty canvas
 * opens the "what can connect here" picker; Cmd+K opens the same picker as a
 * search over every addable type.
 *
 * The component holds no IPC: design mode takes its node-type catalog as the
 * `nodeTypes` prop, and the owning screen supplies it from `useNodeTypes()`.
 * That keeps the canvas fixture-testable, as it has been since P2.1.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
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
  type Connection,
  type Edge,
  type FinalConnectionState,
  type NodeMouseHandler,
} from '@xyflow/react';
import { LayoutGrid } from 'lucide-react';
import '@xyflow/react/dist/style.css';

import { WorkflowNode } from './nodes/WorkflowNode';
import { toFlowGraph, type WorkflowFlowNode } from './flowGraph';
import { useElkLayout } from './useElkLayout';
import { NodeTypePicker, Palette, NODE_TYPE_MIME, type PaletteEntry } from './Palette';
import { atInstanceCap, canConnect, connectableTypesFrom } from './connectRules';
import { addNode, connectNodes, moveNodes, removeEdge, removeNode } from './graphEdits';
import { byKind, type NodeTypeInfo } from './nodeCatalog';
import type { GraphDiff } from './graphDiff';
import type { LintIndex } from './lint';
import type { NodeRunStatus, PositionV2, WorkflowDefinitionV2 } from './types';

/** Below this node count the minimap is noise, so it auto-hides (PRD §6.1). */
const MINIMAP_THRESHOLD = 8;

const NODE_TYPES = { workflow: WorkflowNode };

export type CanvasMode = 'run' | 'design';

export interface WorkflowCanvasProps {
  definition: WorkflowDefinitionV2;
  /** `run` (default) renders read-only with the status overlay; `design`
   *  turns on editing and requires `nodeTypes` + `onDefinitionChange`. */
  mode?: CanvasMode;
  /** Registry catalog from `node_types_list` — the palette's only source,
   *  so a node type added in Rust appears here untouched (PRD §6.3). */
  nodeTypes?: NodeTypeInfo[];
  /** Design mode: called with the next definition after every edit. The
   *  canvas is controlled — it renders whatever comes back. */
  onDefinitionChange?: (next: WorkflowDefinitionV2) => void;
  /** Design mode: a gesture the rules refused, for the owner to toast.
   *  Fires only for *explicit* rejections (a completed drop), never while
   *  React Flow probes candidate targets mid-drag. */
  onConnectRejected?: (message: string) => void;
  /** node id → live run state for the run-mode overlay (P2.2). */
  statusByNode?: Record<string, NodeRunStatus>;
  /** Fired on click or Enter over a node — the panel-open seam (P2.3). */
  onNodeActivate?: (nodeId: string) => void;
  /** Externally-controlled selection (P2.3): when provided, the canvas
   *  highlight follows it — notably so closing the drill-down panel
   *  (`null`) clears the highlight. Omit to let the canvas own selection. */
  selectedNodeId?: string | null;
  /** Node ids in the replay cone to ring before confirming (P2.4). */
  highlightedNodeIds?: Set<string> | null;
  /** Structural-lint findings to badge nodes / tint edges with (P3.3). The
   *  owning screen runs `useWorkflowLint` and passes the result, keeping the
   *  canvas IPC-free. */
  lint?: LintIndex;
  /** Version comparison overlay (P3.4). Pass alongside the *merged* graph
   *  from `mergeForDiff` — and in `run` mode, since the merged graph is a
   *  read-only view of two versions, not something to edit. */
  diff?: GraphDiff;
  className?: string;
}

/** Which overlay picker is open, and why. */
type PickerState =
  | { kind: 'search' }
  | { kind: 'connect'; fromNodeId: string; at: PositionV2; flowAt: PositionV2 };

function CanvasInner({
  definition,
  mode = 'run',
  nodeTypes,
  onDefinitionChange,
  onConnectRejected,
  statusByNode,
  onNodeActivate,
  selectedNodeId,
  highlightedNodeIds,
  lint,
  diff,
  className = '',
}: WorkflowCanvasProps) {
  const design = mode === 'design';
  const catalog = useMemo(() => nodeTypes ?? [], [nodeTypes]);
  const typesByKind = useMemo(() => byKind(catalog), [catalog]);
  const [picker, setPicker] = useState<PickerState | null>(null);
  const base = useMemo(
    () =>
      toFlowGraph(definition, {
        statusByNode,
        highlightedNodeIds,
        showEssence: design,
        lint,
        diff,
      }),
    [definition, statusByNode, highlightedNodeIds, design, lint, diff],
  );

  const [nodes, setNodes, onNodesChange] = useNodesState(base.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(base.edges);
  const { fitView, getNodes, screenToFlowPosition } = useReactFlow();
  const { layout, running } = useElkLayout();
  const wrapperRef = useRef<HTMLDivElement>(null);

  // Re-seed when the definition (or its overlay) changes identity. Positions
  // come from the definition, so this also discards any prior elk layout —
  // intended: a new definition owns its own layout.
  useEffect(() => {
    setNodes(base.nodes);
    setEdges(base.edges);
  }, [base, setNodes, setEdges]);

  // Reflect externally-controlled selection onto the node `selected` flag.
  // Only touches nodes whose flag disagrees, so it's a no-op reconcile after a
  // click (the parent echoes the same id back) and a clear when the panel closes.
  useEffect(() => {
    if (selectedNodeId === undefined) return; // uncontrolled — canvas owns it
    setNodes((prev) =>
      prev.map((n) =>
        n.selected === (n.id === selectedNodeId)
          ? n
          : { ...n, selected: n.id === selectedNodeId },
      ),
    );
  }, [selectedNodeId, setNodes]);

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

  // ── Design mode ────────────────────────────────────────────────────────

  const commit = useCallback(
    (next: WorkflowDefinitionV2) => onDefinitionChange?.(next),
    [onDefinitionChange],
  );

  /** Palette entries: every catalog type, disabled at its instance cap. */
  const paletteEntries: PaletteEntry[] = useMemo(
    () =>
      catalog.map((type) => ({
        type,
        disabledReason: atInstanceCap(definition, type)
          ? `Only ${type.max_instances} ${type.label} node per workflow.`
          : undefined,
      })),
    [catalog, definition],
  );

  /** Where a new node lands when it isn't dropped at a point: just below
   *  the lowest node, so click-to-add never stacks on top of the graph. */
  const spawnPosition = useCallback((): PositionV2 => {
    const lowest = definition.nodes.reduce(
      (acc, n) => (n.position && n.position.y > acc.y ? n.position : acc),
      { x: 0, y: -160 },
    );
    return { x: lowest.x, y: lowest.y + 160 };
  }, [definition.nodes]);

  const addTypeAt = useCallback(
    (type: NodeTypeInfo, position: PositionV2, connectFrom?: string | null) => {
      const { def: next, nodeId } = addNode(definition, type, position, connectFrom);
      commit(next);
      onNodeActivate?.(nodeId);
    },
    [definition, commit, onNodeActivate],
  );

  // React Flow calls this while probing every candidate target mid-drag, so
  // it must stay silent — it only decides whether the handle shows as valid.
  const isValidConnection = useCallback(
    (conn: Connection | Edge) =>
      Boolean(conn.source && conn.target) &&
      canConnect(definition, typesByKind, conn.source!, conn.target!).ok,
    [definition, typesByKind],
  );

  const onConnect = useCallback(
    (conn: Connection) => {
      if (!conn.source || !conn.target) return;
      const verdict = canConnect(definition, typesByKind, conn.source, conn.target);
      if (!verdict.ok) {
        onConnectRejected?.(verdict.message);
        return;
      }
      commit(connectNodes(definition, conn.source, conn.target));
    },
    [definition, typesByKind, commit, onConnectRejected],
  );

  /** Dropping a connection on empty canvas opens the type-compatible
   *  "what can connect here" picker at the drop point (PRD §6.3). */
  const onConnectEnd = useCallback(
    (evt: MouseEvent | TouchEvent, state: FinalConnectionState) => {
      if (!design || state.toNode || !state.fromNode) return;
      const point = 'clientX' in evt ? evt : evt.changedTouches[0];
      if (!point) return;
      const rect = wrapperRef.current?.getBoundingClientRect();
      setPicker({
        kind: 'connect',
        fromNodeId: state.fromNode.id,
        at: { x: point.clientX - (rect?.left ?? 0), y: point.clientY - (rect?.top ?? 0) },
        flowAt: screenToFlowPosition({ x: point.clientX, y: point.clientY }),
      });
    },
    [design, screenToFlowPosition],
  );

  const onDrop = useCallback(
    (evt: React.DragEvent<HTMLDivElement>) => {
      const kind = evt.dataTransfer.getData(NODE_TYPE_MIME);
      const type = catalog.find((t) => t.kind === kind);
      if (!type) return;
      evt.preventDefault();
      if (atInstanceCap(definition, type)) {
        onConnectRejected?.(`Only ${type.max_instances} ${type.label} node per workflow.`);
        return;
      }
      addTypeAt(type, screenToFlowPosition({ x: evt.clientX, y: evt.clientY }));
    },
    [catalog, definition, addTypeAt, screenToFlowPosition, onConnectRejected],
  );

  const onDragOver = useCallback((evt: React.DragEvent<HTMLDivElement>) => {
    if (evt.dataTransfer.types.includes(NODE_TYPE_MIME)) {
      evt.preventDefault();
      evt.dataTransfer.dropEffect = 'copy';
    }
  }, []);

  // Persist a finished drag back into the definition, so layout co-persists
  // with the graph (PRD §5.1) instead of living only in canvas state.
  const onNodeDragStop = useCallback(() => {
    const positions: Record<string, PositionV2> = {};
    for (const n of getNodes()) positions[n.id] = { x: n.position.x, y: n.position.y };
    commit(moveNodes(definition, positions));
  }, [getNodes, commit, definition]);

  const onNodesDelete = useCallback(
    (deleted: { id: string }[]) => {
      commit(
        deleted.reduce<WorkflowDefinitionV2>((acc, n) => removeNode(acc, n.id), definition),
      );
    },
    [commit, definition],
  );

  const onEdgesDelete = useCallback(
    (deleted: Edge[]) => {
      commit(
        deleted.reduce<WorkflowDefinitionV2>(
          (acc, e) => removeEdge(acc, e.source, e.target),
          definition,
        ),
      );
    },
    [commit, definition],
  );

  /** The picker's list: every addable type for Cmd+K, only the
   *  type-compatible ones for a connect-drop. */
  const pickerEntries: PaletteEntry[] = useMemo(() => {
    if (!picker) return [];
    if (picker.kind === 'search') return paletteEntries;
    const allowed = new Set(
      connectableTypesFrom(definition, catalog, picker.fromNodeId).map((t) => t.kind),
    );
    return paletteEntries.filter((e) => allowed.has(e.type.kind));
  }, [picker, paletteEntries, definition, catalog]);

  const onPickerSelect = useCallback(
    (type: NodeTypeInfo) => {
      if (!picker) return;
      if (picker.kind === 'connect') {
        addTypeAt(type, picker.flowAt, picker.fromNodeId);
      } else {
        addTypeAt(type, spawnPosition());
      }
      setPicker(null);
    },
    [picker, addTypeAt, spawnPosition],
  );

  // Arrow keys move the selection to the nearest node in that direction;
  // Enter activates it. Built on node positions so it works for both the
  // migrated vertical chains and future fanned-out layouts.
  const onKeyDown = useCallback(
    (evt: React.KeyboardEvent<HTMLDivElement>) => {
      // Cmd/Ctrl+K opens in-canvas node search (PRD §6.3). Checked before
      // the empty-graph bail-out, since a blank canvas is exactly when you
      // most want it.
      if (design && evt.key.toLowerCase() === 'k' && (evt.metaKey || evt.ctrlKey)) {
        evt.preventDefault();
        setPicker({ kind: 'search' });
        return;
      }

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
    [design, getNodes, activate, onNodeActivate],
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
      onDrop={design ? onDrop : undefined}
      onDragOver={design ? onDragOver : undefined}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        nodeTypes={NODE_TYPES}
        // Run mode is a read-only view: nodes stay selectable so click +
        // keyboard navigation land somewhere, but nothing can be moved or
        // rewired. Design mode turns all of it on.
        nodesConnectable={design}
        nodesDraggable={design}
        nodesFocusable
        edgesFocusable={design}
        deleteKeyCode={design ? ['Backspace', 'Delete'] : null}
        isValidConnection={design ? isValidConnection : undefined}
        onConnect={design ? onConnect : undefined}
        onConnectEnd={design ? onConnectEnd : undefined}
        onNodeDragStop={design ? onNodeDragStop : undefined}
        onNodesDelete={design ? onNodesDelete : undefined}
        onEdgesDelete={design ? onEdgesDelete : undefined}
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
        {design && (
          <Panel position="top-left">
            <Palette
              entries={paletteEntries}
              onSelect={(type) => addTypeAt(type, spawnPosition())}
            />
          </Panel>
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

      {design && picker && (
        <NodeTypePicker
          title={
            picker.kind === 'connect' ? 'Connect to a new node' : 'Add a node'
          }
          entries={pickerEntries}
          anchor={picker.kind === 'connect' ? picker.at : null}
          onSelect={onPickerSelect}
          onDismiss={() => setPicker(null)}
        />
      )}
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
