/**
 * The schema-form core (task P3.2, PRD §6.3): turn a node type's published
 * JSON Schema into an ordered list of field descriptors, and apply an edit
 * back onto the node's opaque `config` payload.
 *
 * This is deliberately a *pure* module. `ConfigPanel.tsx` decides what each
 * descriptor looks like on screen; everything about which fields exist, what
 * control each one needs, and how a value is written back lives here, where it
 * can be tested against the real registry schemas without mounting React.
 *
 * **Why derive rather than enumerate.** The panel must render a node type it
 * has never heard of — that is the same guarantee P3.1 made for the palette,
 * and the reason P3.5's `command` node can land as a registry-only backend
 * diff. So nothing here branches on a `kind`. The one concession to taste is
 * `CODE_FIELD_HINT`: a *key-name* heuristic that promotes long-form prose
 * (`prompt_template`, verifier `instructions`) to a full-height Monaco editor
 * rather than a one-line input. It degrades to a plain textarea for a field it
 * doesn't recognise, so an unknown type is never unusable — only less pretty.
 */
import type { NodeConfigV2, RetryPolicy, RetryRule, RetryStrategy } from './types';

/** Which control renders a field. */
export type FieldControl =
  | 'text' // single-line string
  | 'code' // full-height Monaco (long-form prose / templates)
  | 'boolean'
  | 'integer'
  | 'enum'
  | 'json'; // object/array with no published inner shape

export interface EnumOption {
  /** The `<option>` value — always a string, because that is what the DOM
   *  gives back on change. Use `literal` to write the edit through. */
  value: string | null;
  label: string;
  /** The schema's own value for this choice, at its original type. A schema
   *  may enumerate numbers or booleans (`enum: [1, 2, 3]`), and writing the
   *  stringified `"1"` back into `config` would fail the published schema at
   *  the save boundary. `null` for the "inherit / unset" choice. */
  literal: unknown;
}

export interface SchemaField {
  key: string;
  label: string;
  control: FieldControl;
  description?: string;
  /** Populated for `control: 'enum'`. */
  options?: EnumOption[];
  /** The schema admits `null` — so clearing writes `null` instead of dropping
   *  the key, which is what the migration's own output looks like. */
  nullable: boolean;
  default?: unknown;
  minimum?: number;
  /** `control: 'json'` only — drives the placeholder (`{}` vs `[]`). */
  jsonShape?: 'object' | 'array';
}

/** Key names whose value is long-form prose, not a label. Deliberately narrow:
 *  a wrong guess costs a one-line field a Monaco editor, which is worse than
 *  the reverse, so only genuinely multi-line concepts are listed. */
const CODE_FIELD_HINT = /(prompt|template|instructions|body)/i;

/** Sub-forms `ConfigPanel` renders itself; the generic renderer skips them. */
export const STRUCTURED_CONFIG_KEYS = new Set(['verifier']);

/** JSON Schema's `type` as a set, tolerating both `"string"` and `["string","null"]`. */
function typesOf(schema: Record<string, unknown>): Set<string> {
  const raw = schema.type;
  if (typeof raw === 'string') return new Set([raw]);
  if (Array.isArray(raw)) return new Set(raw.filter((t): t is string => typeof t === 'string'));
  return new Set<string>();
}

/** `agent_kind` → `Agent kind`. Sentence case, not Title Case: these read as
 *  prose labels beside their description, not as headings. */
export function humanizeKey(key: string): string {
  const words = key.replace(/[_-]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function enumOptions(schema: Record<string, unknown>, nullable: boolean): EnumOption[] {
  const values = Array.isArray(schema.enum) ? schema.enum : [];
  const options: EnumOption[] = [];
  let sawNull = false;
  for (const v of values) {
    if (v === null) {
      sawNull = true;
      continue;
    }
    options.push({ value: String(v), label: humanizeKey(String(v)), literal: v });
  }
  // The "unset" choice leads, so the inherit-by-default posture is the first
  // thing the author sees rather than something they scroll past.
  if (sawNull || nullable) {
    options.unshift({ value: null, label: 'Inherit / unset', literal: null });
  }
  return options;
}

/** The schema value behind an `<option>`'s string, for writing an edit back at
 *  its original type. Unknown strings pass through unchanged — a stored value
 *  the schema no longer enumerates must not be silently rewritten. */
export function enumLiteral(field: SchemaField, selected: string | null): unknown {
  const match = field.options?.find((o) => o.value === selected);
  return match ? match.literal : selected;
}

function controlFor(
  key: string,
  schema: Record<string, unknown>,
  types: Set<string>,
): FieldControl {
  if (Array.isArray(schema.enum) && schema.enum.length > 0) return 'enum';
  if (types.has('object') || types.has('array')) return 'json';
  if (types.has('boolean')) return 'boolean';
  if (types.has('integer') || types.has('number')) return 'integer';
  if (types.has('string')) return CODE_FIELD_HINT.test(key) ? 'code' : 'text';
  // A property with no usable `type` (or one we don't model) still has to be
  // editable, or the field would silently vanish from the panel.
  return 'json';
}

/**
 * Compact controls before tall ones. The registry serializes `properties` from
 * a `BTreeMap`, so they arrive alphabetically — which would bury a node's
 * prompt between `model` and `verifier`. Grouping by control keeps the
 * scannable fields together at the top without the frontend having to know a
 * single key name.
 */
const CONTROL_ORDER: Record<FieldControl, number> = {
  enum: 0,
  text: 0,
  integer: 0,
  boolean: 1,
  json: 2,
  code: 3,
};

/**
 * Every editable field the schema publishes, ordered for the panel.
 * Structured sub-forms (`verifier`) are excluded — `ConfigPanel` renders those
 * itself and the generic renderer must not also emit a JSON blob for them.
 */
export function fieldsFromSchema(schema: unknown): SchemaField[] {
  if (!schema || typeof schema !== 'object') return [];
  const props = (schema as Record<string, unknown>).properties;
  if (!props || typeof props !== 'object') return [];

  const fields: SchemaField[] = [];
  for (const [key, rawSpec] of Object.entries(props as Record<string, unknown>)) {
    if (STRUCTURED_CONFIG_KEYS.has(key)) continue;
    const spec = (rawSpec && typeof rawSpec === 'object' ? rawSpec : {}) as Record<
      string,
      unknown
    >;
    const types = typesOf(spec);
    const nullable = types.has('null') || (Array.isArray(spec.enum) && spec.enum.includes(null));
    const control = controlFor(key, spec, types);
    fields.push({
      key,
      label: humanizeKey(key),
      control,
      description: typeof spec.description === 'string' ? spec.description : undefined,
      options: control === 'enum' ? enumOptions(spec, nullable) : undefined,
      nullable,
      default: spec.default,
      minimum: typeof spec.minimum === 'number' ? spec.minimum : undefined,
      jsonShape: types.has('array') ? 'array' : control === 'json' ? 'object' : undefined,
    });
  }

  fields.sort((a, b) => CONTROL_ORDER[a.control] - CONTROL_ORDER[b.control]);
  return fields;
}

/**
 * Write `value` into a node's `config`, returning a new config object.
 *
 * Clearing a field is the interesting case. When the schema admits `null` we
 * write `null` rather than dropping the key: that is exactly what
 * `migrate_v1_to_v2` emits for an unset override, so an edited node stays
 * shape-identical to a migrated one. When it doesn't, the key is removed —
 * writing `null` into a non-nullable field would fail the published schema at
 * the `workflow_update` boundary (P1.3).
 */
export function setConfigValue(
  config: Record<string, unknown> | undefined,
  field: SchemaField,
  value: unknown,
): Record<string, unknown> {
  const next = { ...(config ?? {}) };
  const cleared = value === null || value === undefined || value === '';
  if (cleared) {
    if (field.nullable) next[field.key] = null;
    else delete next[field.key];
  } else {
    next[field.key] = value;
  }
  return next;
}

/** Parse a `json` control's text. Returns the value, or an error to show
 *  inline — the panel keeps the raw text so a half-typed edit isn't lost. */
export function parseJsonField(text: string): { value?: unknown; error?: string } {
  const trimmed = text.trim();
  if (!trimmed) return { value: null };
  try {
    return { value: JSON.parse(trimmed) };
  } catch (err) {
    return { error: err instanceof Error ? err.message : String(err) };
  }
}

/** Render a stored value back into a `json` control's textarea. */
export function jsonFieldText(value: unknown): string {
  if (value === null || value === undefined) return '';
  return JSON.stringify(value, null, 2);
}

// ── Retry policy sub-form ────────────────────────────────────────────────

/** The failure classes the policy engine (P1.10) keys rules by, in the order
 *  the panel lists them: most-authored first. */
export const FAILURE_CLASSES = [
  'verdict',
  'agent_failure',
  'environment',
  'non_retryable',
] as const;

export type FailureClass = (typeof FAILURE_CLASSES)[number];

export const FAILURE_CLASS_LABELS: Record<FailureClass, string> = {
  verdict: 'Verifier said FAIL',
  agent_failure: 'Agent failed',
  environment: 'Environment error',
  non_retryable: 'Non-retryable',
};

export const RETRY_STRATEGY_LABELS: Record<RetryStrategy, string> = {
  in_place: 'Retry in place',
  redirect: 'Send back to a node',
  fail: 'Fail the run',
};

/** The rule a newly-enabled class starts from: the shape the v1→v2 migration
 *  produces for an `on_failure` step, so enabling a class by hand and
 *  migrating one from v1 land on the same JSON. */
export function defaultRetryRule(strategy: RetryStrategy): RetryRule {
  if (strategy === 'redirect') {
    return { strategy, max_attempts: 3, feedback: true, redirect_to: null };
  }
  if (strategy === 'in_place') return { strategy, max_attempts: 3, feedback: true };
  return { strategy };
}

/** Set (or clear, with `null`) one class's rule, pruning the policy back to
 *  `null` once nothing is left so an untouched node stays untouched. */
export function setRetryRule(
  policy: RetryPolicy | null | undefined,
  cls: FailureClass,
  rule: RetryRule | null,
): RetryPolicy | null {
  const next: RetryPolicy = { ...(policy ?? {}) };
  if (rule) next[cls] = rule;
  else delete next[cls];
  return Object.values(next).some((r) => r) ? next : null;
}

/**
 * The card's one-line retry summary — PRD §6.3's `verdict→implement ×3`, so a
 * loop-back is visible on the graph without opening the panel.
 */
export function retrySummary(policy: RetryPolicy | null | undefined): string[] {
  if (!policy) return [];
  const out: string[] = [];
  for (const cls of FAILURE_CLASSES) {
    const rule = policy[cls];
    if (!rule) continue;
    const times = rule.max_attempts && rule.max_attempts > 1 ? ` ×${rule.max_attempts}` : '';
    if (rule.strategy === 'redirect') out.push(`${cls}→${rule.redirect_to ?? '?'}${times}`);
    else if (rule.strategy === 'in_place') out.push(`${cls}${times || ' ×1'}`);
    else out.push(`${cls}→fail`);
  }
  return out;
}

/** Nodes a `redirect_to` may name: any *other* node in the graph. Ancestry is
 *  the engine's rule and the Rust lint's to enforce (P1.4's
 *  `redirect-not-ancestor`); the panel offers the full list and lets the lint
 *  surface (P3.3) explain a bad pick rather than hiding the option. */
export function redirectTargets(nodes: NodeConfigV2[], selfId: string): NodeConfigV2[] {
  return nodes.filter((n) => n.id !== selfId);
}
