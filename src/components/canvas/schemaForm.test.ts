/**
 * The schema-form core (task P3.2).
 *
 * These run against `__fixtures__/node_catalog.json` — the **real** registry
 * catalog, emitted by the Rust `catalog_fixture_is_current` test — rather than
 * hand-written stand-ins. That is what makes "the panel renders whatever the
 * registry publishes" a checked claim instead of a hopeful one: a schema
 * change that the renderer can't express fails here, not in a bug report.
 */
import { describe, expect, it } from 'vitest';

import catalog from './__fixtures__/node_catalog.json';
import {
  defaultRetryRule,
  fieldsFromSchema,
  humanizeKey,
  jsonFieldText,
  parseJsonField,
  redirectTargets,
  retrySummary,
  setConfigValue,
  setRetryRule,
  type SchemaField,
} from './schemaForm';
import type { NodeTypeInfo } from './nodeCatalog';
import type { NodeConfigV2 } from './types';

const CATALOG = catalog as unknown as NodeTypeInfo[];
const typeFor = (kind: string) => CATALOG.find((t) => t.kind === kind)!;
const fieldsFor = (kind: string) => fieldsFromSchema(typeFor(kind).config_schema);
const byKey = (fields: SchemaField[], key: string) => fields.find((f) => f.key === key)!;

describe('fieldsFromSchema over the live registry catalog', () => {
  it('emits a control for every published property of every node type', () => {
    for (const type of CATALOG) {
      const props = Object.keys(
        (type.config_schema as { properties?: Record<string, unknown> }).properties ?? {},
      );
      const rendered = new Set(fieldsFromSchema(type.config_schema).map((f) => f.key));
      // `verifier` is the one exclusion: it has a hand-built sub-form.
      for (const key of props) {
        if (key === 'verifier') continue;
        expect(rendered, `${type.kind}.${key}`).toContain(key);
      }
    }
  });

  it('picks a control per property type', () => {
    const agent = fieldsFor('agent');
    expect(byKey(agent, 'model').control).toBe('text');
    expect(byKey(agent, 'allow_network').control).toBe('boolean');
    expect(byKey(agent, 'max_iterations').control).toBe('integer');
    expect(byKey(agent, 'artifacts').control).toBe('json');
    // The key-name heuristic: prose gets Monaco, not a one-line input.
    expect(byKey(agent, 'prompt_template').control).toBe('code');
  });

  it('renders an enum as a select whose first option is "unset"', () => {
    const effort = byKey(fieldsFor('agent'), 'effort');
    expect(effort.control).toBe('enum');
    expect(effort.options?.[0]).toEqual({ value: null, label: 'Inherit / unset' });
    expect(effort.options?.map((o) => o.value)).toEqual([
      null,
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
  });

  it('derives the capability select from the schema, not a frontend list', () => {
    // The Rust schema gained this enum *for* P3.2 — before that the panel
    // would have had to hardcode the four capability classes.
    const capability = byKey(fieldsFor('agent'), 'capability');
    expect(capability.control).toBe('enum');
    expect(capability.options?.map((o) => o.value)).toEqual([
      null,
      'read_only',
      'artifacts',
      'verify',
      'implement',
    ]);
  });

  it('excludes verifier — it has a structured sub-form', () => {
    expect(fieldsFor('agent').map((f) => f.key)).not.toContain('verifier');
  });

  it('orders compact controls before tall ones', () => {
    // The registry serializes properties alphabetically, which would put the
    // prompt between `model` and `verifier`. Grouping by control fixes that
    // without the frontend knowing a single key name.
    const controls = fieldsFor('agent').map((f) => f.control);
    expect(controls[controls.length - 1]).toBe('code');
    expect(controls.indexOf('json')).toBeLessThan(controls.indexOf('code'));
  });

  it('renders a node type it has never heard of', () => {
    const fields = fieldsFromSchema({
      type: 'object',
      properties: {
        command: { type: 'string', description: 'Shell command to run.' },
        timeout_secs: { type: ['integer', 'null'], minimum: 1 },
        idempotent: { type: 'boolean', default: true },
      },
    });
    expect(fields.map((f) => f.key).sort()).toEqual(['command', 'idempotent', 'timeout_secs']);
    expect(byKey(fields, 'timeout_secs').nullable).toBe(true);
    expect(byKey(fields, 'timeout_secs').minimum).toBe(1);
    expect(byKey(fields, 'idempotent').default).toBe(true);
  });

  it('survives a schema with no properties at all', () => {
    expect(fieldsFromSchema({ type: 'object' })).toEqual([]);
    expect(fieldsFromSchema(undefined)).toEqual([]);
  });
});

describe('setConfigValue', () => {
  const agent = fieldsFor('agent');

  it('writes null when the schema is nullable, matching migrated output', () => {
    const next = setConfigValue({ model: 'sonnet' }, byKey(agent, 'model'), '');
    expect(next).toEqual({ model: null });
  });

  it('deletes the key when the schema is not nullable', () => {
    // `allow_network` is a plain boolean — writing null would fail the
    // published schema at the workflow_update boundary (P1.3).
    const next = setConfigValue({ allow_network: true }, byKey(agent, 'allow_network'), '');
    expect(next).not.toHaveProperty('allow_network');
  });

  it('leaves every other key untouched', () => {
    const before = { model: 'sonnet', capability: 'implement', allow_shell: true };
    const next = setConfigValue(before, byKey(agent, 'model'), 'opus');
    expect(next).toEqual({ model: 'opus', capability: 'implement', allow_shell: true });
    expect(before.model).toBe('sonnet'); // pure
  });

  it('tolerates a node with no config yet', () => {
    expect(setConfigValue(undefined, byKey(agent, 'model'), 'opus')).toEqual({ model: 'opus' });
  });
});

describe('json field helpers', () => {
  it('round-trips a value through text', () => {
    const value = [{ name: 'report', capture: { kind: 'last_write_to', path: 'a.md' } }];
    expect(parseJsonField(jsonFieldText(value)).value).toEqual(value);
  });

  it('reports a parse error instead of throwing', () => {
    const { value, error } = parseJsonField('{ oops');
    expect(value).toBeUndefined();
    expect(error).toBeTruthy();
  });

  it('treats empty text as null', () => {
    expect(parseJsonField('   ')).toEqual({ value: null });
  });
});

describe('retry policy', () => {
  it('summarizes a redirect the way the PRD spells it', () => {
    expect(
      retrySummary({
        verdict: { strategy: 'redirect', redirect_to: 'implement', max_attempts: 3 },
      }),
    ).toEqual(['verdict→implement ×3']);
  });

  it('summarizes in-place and fail rules', () => {
    expect(
      retrySummary({
        environment: { strategy: 'in_place', max_attempts: 2 },
        non_retryable: { strategy: 'fail' },
      }),
    ).toEqual(['environment ×2', 'non_retryable→fail']);
  });

  it('has nothing to say about an unset policy', () => {
    expect(retrySummary(null)).toEqual([]);
    expect(retrySummary({})).toEqual([]);
  });

  it('prunes the policy back to null when the last rule is removed', () => {
    const one = setRetryRule(null, 'verdict', defaultRetryRule('in_place'));
    expect(one?.verdict?.max_attempts).toBe(3);
    expect(setRetryRule(one, 'verdict', null)).toBeNull();
  });

  it('starts a redirect rule from the shape the v1 migration produces', () => {
    expect(defaultRetryRule('redirect')).toEqual({
      strategy: 'redirect',
      max_attempts: 3,
      feedback: true,
      redirect_to: null,
    });
    expect(defaultRetryRule('fail')).toEqual({ strategy: 'fail' });
  });

  it('never offers a node itself as its own redirect target', () => {
    const nodes = [
      { id: 'a', type: 'agent', title: 'A' },
      { id: 'b', type: 'agent', title: 'B' },
    ] as NodeConfigV2[];
    expect(redirectTargets(nodes, 'a').map((n) => n.id)).toEqual(['b']);
  });
});

describe('humanizeKey', () => {
  it('reads as prose, not as a slug', () => {
    expect(humanizeKey('agent_kind')).toBe('Agent kind');
    expect(humanizeKey('max_iterations')).toBe('Max iterations');
    expect(humanizeKey('model')).toBe('Model');
  });
});
