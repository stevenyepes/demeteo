/**
 * `ConfigPanel` — the builder's node config side panel (task P3.2, PRD §6.3).
 *
 * Selecting a node in design mode opens this beside the canvas, the same
 * split-panel shape run mode uses for `NodePanel`. **Never a modal**: the
 * graph stays visible and clickable while you edit, because most config
 * decisions (which node a retry redirects to, whether this is the node that
 * needs the verifier) are only answerable while looking at the graph.
 *
 * Three layers, in increasing order of how much the frontend knows:
 *
 *  1. **Schema-derived fields** — `schemaForm.ts` turns the node type's
 *     published `config_schema` into controls. No `kind` is branched on, so a
 *     node type registered in Rust and never mentioned here still gets a
 *     complete, editable panel. This is the P3.1 zero-frontend-edit guarantee
 *     extended from the palette to config.
 *  2. **Structured sub-forms** — `verifier` and the node's `retry` policy get
 *     hand-built forms with real defaults, per the task goal. `retry` isn't in
 *     `config` at all (it's first-class on `NodeConfigV2`), and `verifier`'s
 *     schema publishes no inner shape, so neither can be derived.
 *  3. **Catalog enhancements** — two well-known keys get better *value
 *     sources* than the schema can express: `agent_kind` becomes a select over
 *     the live agent catalog (`list_agents`) instead of free text, and
 *     `effort` is clamped to what the pinned agent actually accepts
 *     (`effortLevelsFor`). Both degrade to the schema's own control when the
 *     catalog hasn't loaded or the key is absent.
 *
 * The panel is controlled: every edit calls `onChange` with a whole new
 * definition, so P3.3's undo/redo gets the same immutable snapshots
 * `graphEdits.ts` already produces.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';
import Editor from '@monaco-editor/react';
import { Maximize2, Minimize2, Plus, ShieldCheck, Trash2, X } from 'lucide-react';

import { FieldLabel } from '../ui/FieldLabel';
import { useAgentCatalog, effortLevelsFor } from '../../lib/agentCatalog';
import { EFFORT_LABELS, isEffortLevel } from '../../lib/effortLevels';
import { TONE_CHIP, TONE_TEXT } from '../../lib/runStatus';
import { MONACO_RESIZE_SAFE } from '../../lib/monaco';
import {
  FAILURE_CLASSES,
  FAILURE_CLASS_LABELS,
  RETRY_STRATEGY_LABELS,
  defaultRetryRule,
  enumLiteral,
  fieldsFromSchema,
  jsonFieldText,
  parseJsonField,
  redirectTargets,
  setConfigValue,
  setRetryRule,
  type FailureClass,
  type SchemaField,
} from './schemaForm';
import { nodeTypeMeta } from './types';
import type { NodeTypeInfo } from './nodeCatalog';
import type {
  JoinSemantics,
  NodeConfigV2,
  RetryRule,
  RetryStrategy,
  WorkflowDefinitionV2,
} from './types';

const INPUT_CLASS =
  'w-full rounded-lg border border-slate-700/60 bg-slate-950/60 px-2.5 py-1.5 text-xs text-slate-200 placeholder:text-slate-600 focus:border-cyan-500/50 focus:outline-none';

const MONACO_OPTIONS = {
  ...MONACO_RESIZE_SAFE,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 12,
  lineNumbers: 'on' as const,
  wordWrap: 'on' as const,
  renderLineHighlight: 'none' as const,
  contextmenu: false,
  overviewRulerLanes: 0,
  overviewRulerBorder: false,
  scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
};

/** The verifier defaults a freshly-enabled verifier starts from — the same
 *  literal `WorkflowEditor` wrote, so a node configured in the old form editor
 *  and one configured here are byte-identical. */
const VERIFIER_DEFAULTS = {
  instructions: 'Verify that the changes are correct and the tests pass.',
  agent_kind: null,
  harness_names: [],
  verdict_key: 'verdict',
};

/** The verifier's harness selection, reading both spellings: `harness_names`
 *  (the ordered list) and the singular `harness_name` it replaced, which any
 *  workflow authored before it still carries. */
function readHarnessNames(v: Record<string, unknown>): string[] {
  if (Array.isArray(v.harness_names)) {
    return v.harness_names.filter((n): n is string => typeof n === 'string');
  }
  return typeof v.harness_name === 'string' && v.harness_name ? [v.harness_name] : [];
}

const JOIN_OPTIONS: { value: JoinSemantics; label: string; hint: string }[] = [
  {
    value: 'all_success',
    label: 'All must succeed',
    hint: 'Runs only when every incoming node completed successfully.',
  },
  {
    value: 'any_success',
    label: 'Any may succeed',
    hint: 'Runs as soon as one incoming node completes successfully.',
  },
  {
    value: 'all_done',
    label: 'All must finish',
    hint: 'Runs once every incoming node reached a terminal state, success or not.',
  },
];

export interface ConfigPanelProps {
  definition: WorkflowDefinitionV2;
  /** The selected node. `null` renders nothing — the owner unmounts instead. */
  nodeId: string;
  /** Registry catalog (`node_types_list`), for the selected node's schema. */
  nodeTypes: NodeTypeInfo[];
  onChange: (next: WorkflowDefinitionV2) => void;
  onClose: () => void;
  className?: string;
}

export function ConfigPanel({
  definition,
  nodeId,
  nodeTypes,
  onChange,
  onClose,
  className = '',
}: ConfigPanelProps) {
  const node = definition.nodes.find((n) => n.id === nodeId) ?? null;
  const type = nodeTypes.find((t) => t.kind === node?.type) ?? null;
  const { agents } = useAgentCatalog();
  /** Which `code` field is expanded to fill the panel, if any. */
  const [expanded, setExpanded] = useState<string | null>(null);

  useEffect(() => setExpanded(null), [nodeId]);

  const fields = useMemo(() => fieldsFromSchema(type?.config_schema), [type]);

  const updateNode = useCallback(
    (patch: Partial<NodeConfigV2>) => {
      onChange({
        ...definition,
        nodes: definition.nodes.map((n) => (n.id === nodeId ? { ...n, ...patch } : n)),
      });
    },
    [definition, nodeId, onChange],
  );

  const setConfig = useCallback(
    (next: Record<string, unknown>) => updateNode({ config: next }),
    [updateNode],
  );

  if (!node) return null;

  const meta = nodeTypeMeta(node.type);
  const TypeIcon = meta.icon;
  const config = node.config ?? {};
  const pinnedAgent = typeof config.agent_kind === 'string' ? config.agent_kind : null;
  const incoming = definition.edges.filter((e) => e.to === node.id).length;
  const expandedField = expanded ? fields.find((f) => f.key === expanded) : null;

  return (
    <div
      className={`flex h-full w-[62%] min-w-0 flex-col border-l border-white/5 bg-[#0d0f14]/80 backdrop-blur-xl ${className}`}
      data-testid="config-panel"
    >
      <div className="flex shrink-0 items-start justify-between gap-3 border-b border-white/5 px-5 py-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <TypeIcon className={`h-4 w-4 shrink-0 ${TONE_TEXT[meta.tone]}`} aria-hidden />
            <h3 className="truncate font-display text-sm font-bold uppercase tracking-wider text-white">
              {node.title || node.id}
            </h3>
          </div>
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <span
              className={`rounded border px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider ${TONE_CHIP[meta.tone]}`}
            >
              {type?.label ?? meta.label}
            </span>
            <span className="font-mono text-[10px] text-slate-500">{node.id}</span>
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          aria-label="Close config panel"
          className="shrink-0 rounded-lg p-1.5 text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* A code field expanded to full height replaces the form body rather
          than opening over it — the point of "Monaco full-height" is the
          height, which an overlay inside a 62% panel would not gain. */}
      {expandedField ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex items-center justify-between border-b border-white/5 px-5 py-2">
            <FieldLabel className="mb-0">{expandedField.label}</FieldLabel>
            <button
              type="button"
              onClick={() => setExpanded(null)}
              className="flex items-center gap-1.5 rounded-lg border border-slate-700/60 px-2 py-1 text-[11px] text-slate-300 transition-colors hover:border-slate-600 hover:text-white"
            >
              <Minimize2 className="h-3 w-3" /> Collapse
            </button>
          </div>
          <div className="min-h-0 flex-1">
            <CodeField
              value={config[expandedField.key]}
              onChange={(v) => setConfig(setConfigValue(config, expandedField, v))}
              height="100%"
            />
          </div>
        </div>
      ) : (
        <div className="min-h-0 flex-1 space-y-5 overflow-y-auto px-5 py-4">
          <Section title="Identity">
            <div>
              <FieldLabel htmlFor="cfg-title">Title</FieldLabel>
              <input
                id="cfg-title"
                type="text"
                value={node.title}
                onChange={(evt) => updateNode({ title: evt.target.value })}
                className={INPUT_CLASS}
              />
            </div>
            {type?.summary && (
              <p className="text-[11px] leading-relaxed text-slate-500">{type.summary}</p>
            )}
          </Section>

          {fields.length > 0 && (
            <Section title="Configuration">
              {fields.map((field) => (
                <GenericField
                  key={field.key}
                  field={field}
                  value={config[field.key]}
                  agentKinds={agents.map((a) => a.kind)}
                  effortLevels={
                    pinnedAgent ? effortLevelsFor(agents, pinnedAgent) : null
                  }
                  onChange={(v) => setConfig(setConfigValue(config, field, v))}
                  onExpand={() => setExpanded(field.key)}
                />
              ))}
            </Section>
          )}

          {supportsVerifier(type) && (
            <VerifierForm
              value={config.verifier as Record<string, unknown> | null | undefined}
              agentKinds={agents.map((a) => a.kind)}
              onChange={(v) => setConfig({ ...config, verifier: v })}
            />
          )}

          <RetryForm node={node} definition={definition} onChange={updateNode} />

          {incoming > 1 && (
            <Section title="Join">
              <p className="text-[11px] leading-relaxed text-slate-500">
                {incoming} nodes feed this one. Choose when it becomes ready.
              </p>
              <select
                aria-label="Join semantics"
                value={node.join ?? definition.defaults?.join ?? 'all_success'}
                onChange={(evt) =>
                  updateNode({ join: evt.target.value as JoinSemantics })
                }
                className={INPUT_CLASS}
              >
                {JOIN_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
              <p className="text-[11px] text-slate-500">
                {JOIN_OPTIONS.find(
                  (o) => o.value === (node.join ?? definition.defaults?.join ?? 'all_success'),
                )?.hint}
              </p>
            </Section>
          )}
        </div>
      )}
    </div>
  );
}

/** A node type accepts a verifier iff its schema publishes the key. Derived,
 *  not hardcoded — `gate` and `sync` simply don't declare it. */
function supportsVerifier(type: NodeTypeInfo | null): boolean {
  const props = (type?.config_schema as Record<string, unknown> | undefined)?.properties;
  return Boolean(props && typeof props === 'object' && 'verifier' in props);
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="space-y-3">
      <h4 className="text-[10px] font-bold uppercase tracking-widest text-slate-500">{title}</h4>
      {children}
    </section>
  );
}

// ── Generic, schema-derived field ────────────────────────────────────────

interface GenericFieldProps {
  field: SchemaField;
  value: unknown;
  /** Live agent catalog, for the `agent_kind` value-source enhancement. */
  agentKinds: string[];
  /** Effort levels the pinned agent accepts; `null` when none is pinned. */
  effortLevels: readonly string[] | null;
  onChange: (value: unknown) => void;
  onExpand: () => void;
}

function GenericField({
  field,
  value,
  agentKinds,
  effortLevels,
  onChange,
  onExpand,
}: GenericFieldProps) {
  const id = `cfg-${field.key}`;

  return (
    <div>
      <div className="flex items-center justify-between">
        {/* Monaco is not a labelable control, so a `code` field's label points
            at nothing rather than at the wrong element. */}
        <FieldLabel htmlFor={field.control === 'code' ? undefined : id}>{field.label}</FieldLabel>
        {field.control === 'code' && (
          <button
            type="button"
            onClick={onExpand}
            aria-label={`Expand ${field.label}`}
            className="mb-1.5 rounded p-1 text-slate-500 transition-colors hover:bg-white/5 hover:text-slate-200"
            title="Edit full-height"
          >
            <Maximize2 className="h-3 w-3" />
          </button>
        )}
      </div>

      <FieldControlView
        id={id}
        field={field}
        value={value}
        agentKinds={agentKinds}
        effortLevels={effortLevels}
        onChange={onChange}
      />

      {field.description && (
        <p className="mt-1 text-[11px] leading-relaxed text-slate-500">{field.description}</p>
      )}
    </div>
  );
}

function FieldControlView({
  id,
  field,
  value,
  agentKinds,
  effortLevels,
  onChange,
}: Omit<GenericFieldProps, 'onExpand'> & { id: string }) {
  // `agent_kind` has no enum in the schema — the registry can't know which
  // agents this install has. The catalog can, so it supplies the options.
  if (field.key === 'agent_kind' && agentKinds.length > 0) {
    return (
      <select
        id={id}
        value={typeof value === 'string' ? value : ''}
        onChange={(evt) => onChange(evt.target.value || null)}
        className={INPUT_CLASS}
      >
        <option value="">Inherit / unset</option>
        {agentKinds.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
    );
  }

  switch (field.control) {
    case 'enum': {
      // Effort is the one enum the schema over-states: it lists every level
      // the product knows, but a pinned agent may accept fewer.
      const options =
        field.key === 'effort' && effortLevels
          ? (field.options ?? []).filter(
              (o) => o.value === null || effortLevels.includes(o.value),
            )
          : (field.options ?? []);
      return (
        <select
          id={id}
          value={value == null ? '' : String(value)}
          // Through `enumLiteral` rather than the raw `<option>` string: a
          // schema may enumerate numbers or booleans, and the DOM only ever
          // hands back a string.
          onChange={(evt) => onChange(enumLiteral(field, evt.target.value || null))}
          className={INPUT_CLASS}
        >
          {options.map((o) => (
            <option key={o.value ?? ''} value={o.value ?? ''}>
              {o.value && isEffortLevel(o.value) ? EFFORT_LABELS[o.value] : o.label}
            </option>
          ))}
        </select>
      );
    }

    case 'boolean':
      return (
        <label className="flex cursor-pointer items-center gap-2 text-xs text-slate-300">
          <input
            id={id}
            type="checkbox"
            checked={value === true}
            onChange={(evt) => onChange(evt.target.checked)}
            className="h-3.5 w-3.5 accent-violet-500"
          />
          {value === true ? 'Enabled' : 'Disabled'}
        </label>
      );

    case 'integer':
      return (
        <input
          id={id}
          type="number"
          min={field.minimum}
          value={typeof value === 'number' ? value : ''}
          onChange={(evt) =>
            onChange(evt.target.value === '' ? null : Number(evt.target.value))
          }
          className={INPUT_CLASS}
        />
      );

    case 'code':
      return (
        <CodeField
          value={value}
          onChange={onChange}
          height="220px"
          testId={`code-${field.key}`}
        />
      );

    case 'json':
      return <JsonField id={id} field={field} value={value} onChange={onChange} />;

    default:
      return (
        <input
          id={id}
          type="text"
          value={typeof value === 'string' ? value : ''}
          onChange={(evt) => onChange(evt.target.value)}
          className={INPUT_CLASS}
        />
      );
  }
}

/** Monaco for long-form prose. Markdown highlighting: prompt templates are
 *  markdown with `{{placeholder}}` interpolation, which is what every starter
 *  ships today. */
function CodeField({
  value,
  onChange,
  height,
  testId,
}: {
  value: unknown;
  onChange: (v: string) => void;
  height: string;
  testId?: string;
}) {
  return (
    <div
      className="overflow-hidden rounded-lg border border-slate-700/60"
      style={height === '100%' ? { height: '100%' } : undefined}
      data-testid={testId}
    >
      <Editor
        height={height}
        language="markdown"
        theme="vs-dark"
        value={typeof value === 'string' ? value : ''}
        onChange={(next) => onChange(next ?? '')}
        options={MONACO_OPTIONS}
      />
    </div>
  );
}

/**
 * An object/array property whose inner shape the schema doesn't publish
 * (`artifacts` today). Rendering a fabricated form would invent structure the
 * registry never declared — and would silently drop any key it didn't model on
 * save. A JSON editor is the honest control: it round-trips anything, and it
 * is the generic escape hatch for a future node type's complex config.
 */
function JsonField({
  id,
  field,
  value,
  onChange,
}: {
  id: string;
  field: SchemaField;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const [text, setText] = useState(() => jsonFieldText(value));
  const [error, setError] = useState<string | null>(null);

  // Re-seed when the panel retargets, or an outside edit changes the value —
  // but not on every keystroke, or the textarea would fight the user.
  useEffect(() => {
    setText(jsonFieldText(value));
    setError(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [field.key]);

  return (
    <div>
      <textarea
        id={id}
        rows={4}
        spellCheck={false}
        value={text}
        placeholder={field.jsonShape === 'array' ? '[]' : '{}'}
        onChange={(evt) => {
          setText(evt.target.value);
          const parsed = parseJsonField(evt.target.value);
          if (parsed.error) setError(parsed.error);
          else {
            setError(null);
            onChange(parsed.value);
          }
        }}
        className={`${INPUT_CLASS} resize-y font-mono leading-relaxed`}
      />
      {error && <p className="mt-1 text-[11px] text-rose-400">Invalid JSON — {error}</p>}
    </div>
  );
}

// ── Verifier sub-form ────────────────────────────────────────────────────

function VerifierForm({
  value,
  agentKinds,
  onChange,
}: {
  value: Record<string, unknown> | null | undefined;
  agentKinds: string[];
  onChange: (next: Record<string, unknown> | null) => void;
}) {
  const enabled = Boolean(value);
  const v = value ?? {};
  const set = (patch: Record<string, unknown>) => onChange({ ...v, ...patch });

  return (
    <Section title="Verifier">
      <label className="flex cursor-pointer items-center gap-2 text-xs text-slate-300">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(evt) => onChange(evt.target.checked ? { ...VERIFIER_DEFAULTS } : null)}
          className="h-3.5 w-3.5 accent-violet-500"
        />
        <ShieldCheck className="h-3.5 w-3.5 text-emerald-400" aria-hidden />
        Verify this node&rsquo;s output with a second agent turn
      </label>

      {enabled && (
        <div className="space-y-3 border-l border-slate-700/60 pl-3">
          <div>
            <FieldLabel htmlFor="cfg-verifier-instructions">Instructions</FieldLabel>
            <textarea
              id="cfg-verifier-instructions"
              rows={3}
              value={typeof v.instructions === 'string' ? v.instructions : ''}
              onChange={(evt) => set({ instructions: evt.target.value })}
              className={`${INPUT_CLASS} resize-y leading-relaxed`}
            />
          </div>
          <div>
            <FieldLabel htmlFor="cfg-verifier-agent">Verifier agent</FieldLabel>
            <select
              id="cfg-verifier-agent"
              value={typeof v.agent_kind === 'string' ? v.agent_kind : ''}
              onChange={(evt) => set({ agent_kind: evt.target.value || null })}
              className={INPUT_CLASS}
            >
              <option value="">Same as the node&rsquo;s agent</option>
              {agentKinds.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </div>
          <div>
            <FieldLabel htmlFor="cfg-verifier-harness">Harnesses</FieldLabel>
            <input
              id="cfg-verifier-harness"
              type="text"
              value={readHarnessNames(v).join(', ')}
              onChange={(evt) =>
                set({
                  harness_names: evt.target.value
                    .split(',')
                    .map((n) => n.trim())
                    .filter(Boolean),
                  // Drop the singular spelling this replaced, so a node edited
                  // here stops carrying two sources of truth. The backend still
                  // accepts it for workflows nobody has opened.
                  harness_name: undefined,
                })
              }
              placeholder="Project default"
              className={INPUT_CLASS}
            />
            <p className="mt-1 text-[11px] text-slate-500">
              Comma-separated. Each runs separately, in this order, and all of them must pass —
              leave blank to use the project&rsquo;s selected gates.
            </p>
          </div>
          <div>
            <FieldLabel htmlFor="cfg-verifier-key">Verdict key</FieldLabel>
            <input
              id="cfg-verifier-key"
              type="text"
              value={typeof v.verdict_key === 'string' ? v.verdict_key : ''}
              onChange={(evt) => set({ verdict_key: evt.target.value })}
              placeholder="verdict"
              className={INPUT_CLASS}
            />
          </div>
        </div>
      )}
    </Section>
  );
}

// ── Retry policy sub-form ────────────────────────────────────────────────

function RetryForm({
  node,
  definition,
  onChange,
}: {
  node: NodeConfigV2;
  definition: WorkflowDefinitionV2;
  onChange: (patch: Partial<NodeConfigV2>) => void;
}) {
  const policy = node.retry ?? null;
  const targets = redirectTargets(definition.nodes, node.id);

  const setRule = (cls: FailureClass, rule: RetryRule | null) =>
    onChange({ retry: setRetryRule(policy, cls, rule) });

  return (
    <Section title="Retry policy">
      <p className="text-[11px] leading-relaxed text-slate-500">
        What happens when this node fails, per failure class. An unset class
        falls back to the workflow default.
      </p>

      {FAILURE_CLASSES.map((cls) => {
        const rule = policy?.[cls] ?? null;
        return (
          <div
            key={cls}
            className="space-y-2 rounded-lg border border-slate-700/50 bg-slate-900/40 p-2.5"
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-xs font-medium text-slate-300">
                {FAILURE_CLASS_LABELS[cls]}
              </span>
              {rule ? (
                <button
                  type="button"
                  onClick={() => setRule(cls, null)}
                  aria-label={`Remove ${cls} rule`}
                  className="rounded p-1 text-slate-500 transition-colors hover:bg-white/5 hover:text-rose-400"
                >
                  <Trash2 className="h-3 w-3" />
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => setRule(cls, defaultRetryRule('in_place'))}
                  aria-label={`Add ${cls} rule`}
                  className="flex items-center gap-1 rounded border border-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200"
                >
                  <Plus className="h-3 w-3" /> Rule
                </button>
              )}
            </div>

            {rule && (
              <div className="space-y-2">
                <select
                  aria-label={`${cls} strategy`}
                  value={rule.strategy}
                  onChange={(evt) => {
                    const strategy = evt.target.value as RetryStrategy;
                    // Switching strategy resets to that strategy's defaults —
                    // carrying `max_attempts` onto `fail` would persist a
                    // budget the engine will never read.
                    setRule(cls, {
                      ...defaultRetryRule(strategy),
                      ...(strategy === rule.strategy ? rule : {}),
                      strategy,
                    });
                  }}
                  className={INPUT_CLASS}
                >
                  {(Object.keys(RETRY_STRATEGY_LABELS) as RetryStrategy[]).map((s) => (
                    <option key={s} value={s}>
                      {RETRY_STRATEGY_LABELS[s]}
                    </option>
                  ))}
                </select>

                {rule.strategy === 'redirect' && (
                  <select
                    aria-label={`${cls} redirect target`}
                    value={rule.redirect_to ?? ''}
                    onChange={(evt) =>
                      setRule(cls, { ...rule, redirect_to: evt.target.value || null })
                    }
                    className={INPUT_CLASS}
                  >
                    <option value="">Choose a node…</option>
                    {targets.map((t) => (
                      <option key={t.id} value={t.id}>
                        {t.title || t.id}
                      </option>
                    ))}
                  </select>
                )}

                {rule.strategy !== 'fail' && (
                  <div className="flex items-center gap-3">
                    <label className="flex items-center gap-1.5 text-[11px] text-slate-400">
                      Max attempts
                      <input
                        type="number"
                        min={1}
                        aria-label={`${cls} max attempts`}
                        value={rule.max_attempts ?? ''}
                        onChange={(evt) =>
                          setRule(cls, {
                            ...rule,
                            max_attempts:
                              evt.target.value === '' ? null : Number(evt.target.value),
                          })
                        }
                        className={`${INPUT_CLASS} w-16`}
                      />
                    </label>
                    <label className="flex cursor-pointer items-center gap-1.5 text-[11px] text-slate-400">
                      <input
                        type="checkbox"
                        checked={rule.feedback !== false}
                        onChange={(evt) => setRule(cls, { ...rule, feedback: evt.target.checked })}
                        className="h-3.5 w-3.5 accent-violet-500"
                      />
                      Pass the failure back as feedback
                    </label>
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </Section>
  );
}
