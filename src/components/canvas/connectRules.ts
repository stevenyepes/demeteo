/**
 * Connect-time validation for design mode (task P3.1) — the client-side
 * mirror of the Rust structural lint in `domain/workflow_graph.rs` (P1.4).
 *
 * The rules table is **ported, not re-derived**: every verdict below carries
 * the same `code` the Rust lint emits for the equivalent finding, so an edge
 * the editor refuses and a definition the engine refuses fail for the same
 * named reason. Where the two differ it is by design and noted inline:
 *
 * - `cycle`, `self-edge` → Rust `cycle` (a self-edge is its 1-node case).
 *   Prevented at connect time here so the author never builds one.
 * - `port-type-mismatch` → same code. The Rust lint reads *per-node* declared
 *   ports (`config.inputs` / `config.outputs`); this adds the **type-level**
 *   defaults from the registry (`node_types_list`) as the fallback, so a
 *   fresh node with no declared ports is still checked.
 * - `finalize-not-sink` → same code, expressed structurally: `finalize`
 *   declares no outputs, so nothing is compatible downstream of it.
 * - `multiple-finalize` → same code, enforced at *add* time via the type's
 *   `max_instances` rather than as an after-the-fact finding.
 * - `duplicate-edge` is editor-only: the engine tolerates a repeated edge
 *   (adjacency is a set walk), but React Flow needs unique edge ids and a
 *   double line is never what the author meant.
 *
 * Everything here is pure so it can be exhaustively unit tested without
 * mounting React Flow — the same seam `flowGraph.ts` and `graphOps.ts` use.
 */
import type { NodeTypeInfo, PortType } from './nodeCatalog';
import type { NodeConfigV2, WorkflowDefinitionV2 } from './types';

/** Rust `PortType::compatible_with`: equal types connect, `any` connects
 *  with everything on either side. */
export function portsCompatible(a: PortType, b: PortType): boolean {
  return a === 'any' || b === 'any' || a === b;
}

/**
 * The ports a specific node presents. A node may narrow its type's defaults
 * by declaring `config.outputs` / `config.inputs` as `[{ name, type }]` —
 * the shape `declared_ports` parses in the Rust lint. Anything unparseable
 * falls back to the type default rather than silently blocking connections.
 */
export function effectivePorts(
  node: NodeConfigV2,
  type: NodeTypeInfo | undefined,
): { inputs: PortType[]; outputs: PortType[] } {
  return {
    inputs: declaredPorts(node, 'inputs') ?? type?.inputs ?? ['any'],
    outputs: declaredPorts(node, 'outputs') ?? type?.outputs ?? ['any'],
  };
}

function declaredPorts(node: NodeConfigV2, key: 'inputs' | 'outputs'): PortType[] | null {
  const raw = node.config?.[key];
  if (!Array.isArray(raw)) return null;
  const types = raw
    .map((entry) =>
      entry && typeof entry === 'object' ? (entry as { type?: unknown }).type : undefined,
    )
    .filter((t): t is PortType => typeof t === 'string');
  // An `outputs: []` declaration is meaningful ("sink"), but a list we could
  // parse nothing out of is not — treat that as undeclared.
  return raw.length > 0 && types.length === 0 ? null : types;
}

/** Machine codes, matching the Rust lint vocabulary where one exists. */
export type ConnectRejectionCode =
  | 'self-edge'
  | 'cycle'
  | 'duplicate-edge'
  | 'port-type-mismatch'
  | 'unknown-node';

export type ConnectVerdict =
  | { ok: true }
  | { ok: false; code: ConnectRejectionCode; message: string };

const OK: ConnectVerdict = { ok: true };

/** Does `to` already reach `from`? Then `from → to` would close a cycle. */
function reaches(def: WorkflowDefinitionV2, from: string, to: string): boolean {
  const out = new Map<string, string[]>();
  for (const e of def.edges) {
    const list = out.get(e.from);
    if (list) list.push(e.to);
    else out.set(e.from, [e.to]);
  }
  const seen = new Set<string>([from]);
  const stack = [from];
  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur === to) return true;
    for (const next of out.get(cur) ?? []) {
      if (!seen.has(next)) {
        seen.add(next);
        stack.push(next);
      }
    }
  }
  return false;
}

/**
 * May an edge `from → to` be created in `def`? Returns the first violation,
 * carrying a message written for a connect-time toast (so it names the nodes,
 * not just the rule).
 */
export function canConnect(
  def: WorkflowDefinitionV2,
  types: Map<string, NodeTypeInfo>,
  from: string,
  to: string,
): ConnectVerdict {
  if (from === to) {
    return {
      ok: false,
      code: 'self-edge',
      message: 'A node cannot depend on itself.',
    };
  }

  const source = def.nodes.find((n) => n.id === from);
  const target = def.nodes.find((n) => n.id === to);
  if (!source || !target) {
    return {
      ok: false,
      code: 'unknown-node',
      message: `Edge references a node that is not on the canvas ('${!source ? from : to}').`,
    };
  }

  if (def.edges.some((e) => e.from === from && e.to === to)) {
    return {
      ok: false,
      code: 'duplicate-edge',
      message: `'${source.title}' already feeds '${target.title}'.`,
    };
  }

  // `to` already reaching `from` means this edge would close a loop. The
  // definition graph stays acyclic by construction — iteration lives in run
  // state (retry redirects), never in the topology.
  if (reaches(def, to, from)) {
    return {
      ok: false,
      code: 'cycle',
      message: `That would create a loop back to '${source.title}'. Use a retry redirect instead of a cycle.`,
    };
  }

  const outputs = effectivePorts(source, types.get(source.type)).outputs;
  const inputs = effectivePorts(target, types.get(target.type)).inputs;
  if (outputs.length === 0) {
    // `finalize` — the branch is squashed and published, so nothing may run
    // after it (`finalize-not-sink`).
    return {
      ok: false,
      code: 'port-type-mismatch',
      message: `'${source.title}' ends the run — nothing can run after it.`,
    };
  }
  if (inputs.length === 0 || !outputs.some((o) => inputs.some((i) => portsCompatible(o, i)))) {
    return {
      ok: false,
      code: 'port-type-mismatch',
      message: `'${source.title}' produces ${outputs.join(', ')}, which '${target.title}' does not accept (${inputs.join(', ') || 'nothing'}).`,
    };
  }

  return OK;
}

/**
 * Which node types may be added at the cap — the palette's enable/disable
 * state. A type with `max_instances` already met is offered greyed out
 * rather than hidden, so the author learns the rule instead of wondering
 * where the entry went.
 */
export function atInstanceCap(def: WorkflowDefinitionV2, type: NodeTypeInfo): boolean {
  if (type.max_instances == null) return false;
  return def.nodes.filter((n) => n.type === type.kind).length >= type.max_instances;
}

/**
 * The "what can connect here" picker (PRD §6.3): dragging from a node's
 * output handle into empty canvas offers only the types that could legally
 * receive that output *and* are not at their instance cap.
 *
 * A brand-new node has no declared ports, so its type defaults decide —
 * which is exactly why the registry publishes them.
 */
export function connectableTypesFrom(
  def: WorkflowDefinitionV2,
  nodeTypes: NodeTypeInfo[],
  fromNodeId: string,
): NodeTypeInfo[] {
  const types = new Map(nodeTypes.map((t) => [t.kind, t]));
  const source = def.nodes.find((n) => n.id === fromNodeId);
  if (!source) return [];
  const outputs = effectivePorts(source, types.get(source.type)).outputs;
  if (outputs.length === 0) return []; // a sink connects to nothing

  return nodeTypes.filter(
    (t) =>
      !atInstanceCap(def, t) &&
      t.inputs.length > 0 &&
      outputs.some((o) => t.inputs.some((i) => portsCompatible(o, i))),
  );
}
