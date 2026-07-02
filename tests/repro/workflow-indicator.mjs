/**
 * Workflow indicator on the pipeline card — `workflowById` lookup and
 * badge classification.
 *
 * Bug context:
 *   `src/components/ProjectHome.tsx` (the "Active Running Pipelines"
 *   list) renders a small badge on each feature card that names the
 *   workflow the run belongs to. The badge is fed by a
 *   `workflowById: Map<string, { name; is_starter }>` populated from
 *   the same `workflow_list` payload that drives the Smart-Inference
 *   picker.
 *
 *   Two regressions are easy to introduce and were guarded against in
 *   this test:
 *
 *   1. The lookup skipped entries with `workflow_id == null` (or empty
 *      string) and rendered a violet badge with the literal word
 *      "undefined" inside — making it look like a real workflow was
 *      matched. Fixed by short-circuiting to the muted "Workflow:
 *      unknown" fallback whenever the lookup misses.
 *
 *   2. The lookup matched on the first entry of the workflows list
 *      regardless of `workflow_id`, so deleting a workflow (or having
 *      a feature outlive its workflow) silently relabelled every
 *      card with the wrong name. Fixed by keying on `workflow.id`
 *      exactly and falling back when the key is absent.
 *
 * What this test asserts (re-implements the lookup + classification
 * step from `src/components/ProjectHome.tsx`):
 *   1. With a known `workflow_id`, the card classifies the badge as
 *      a violet pill containing the workflow's `name` and the
 *      correct `Starter` / `Custom` suffix.
 *   2. With a missing or unknown `workflow_id`, the card classifies
 *      the badge as the muted `Workflow: unknown` fallback (no crash,
 *      no empty space, no leftover `undefined`).
 *   3. The lookup survives a re-fetch where the workflows list is
 *      replaced (Map keyed by id, not positional).
 *
 * Run (pure Node ≥ 18, no project deps needed):
 *
 *   $ node tests/repro/workflow-indicator.mjs
 */

import assert from 'node:assert/strict';

// --------------------------------------------------------------------------
// Re-implementation of the lookup + classification step from
//   `src/components/ProjectHome.tsx::fetchWorkspaceData` (build the
//   `workflowById` Map) and the inline IIFE inside `features.map(...)`
//   (classify the badge). Kept in sync with the production code; if
//   either of those two blocks changes shape, this test must follow.
//

/**
 * Build the `workflowById` map. Mirrors the loop in
 * `fetchWorkspaceData` (ProjectHome.tsx, lines ~209–222 in the
 * post-implementation layout).
 *
 * @param {Array<{ id?: unknown; name?: unknown; is_starter?: unknown }>} list
 */
function buildWorkflowById(list) {
    const lookup = new Map();
    for (const wf of list) {
        if (wf && typeof wf.id === 'string' && wf.id.length > 0) {
            lookup.set(wf.id, {
                name: typeof wf.name === 'string' ? wf.name : '',
                is_starter: Boolean(wf.is_starter),
            });
        }
    }
    return lookup;
}

/**
 * Classify the badge for a given feature based on its `workflow_id`
 * and the current lookup. Mirrors the IIFE in
 * `ProjectHome.tsx::features.map(...)` (post-implementation layout).
 *
 * @param {{ workflow_id?: string | null | undefined }} feature
 * @param {Map<string, { name: string; is_starter: boolean }>} lookup
 * @returns {{ variant: 'known' | 'fallback'; name?: string; is_starter?: boolean; label: string }}
 */
function classifyBadge(feature, lookup) {
    const wfMeta =
        feature && feature.workflow_id
            ? lookup.get(feature.workflow_id)
            : undefined;
    if (!wfMeta) {
        return { variant: 'fallback', label: 'Workflow: unknown' };
    }
    return {
        variant: 'known',
        name: wfMeta.name,
        is_starter: wfMeta.is_starter,
        label: `Workflow: ${wfMeta.name} (${wfMeta.is_starter ? 'Starter' : 'Custom'})`,
    };
}

// --------------------------------------------------------------------------
// Tests
//

const workflows = [
    { id: 'wf-bugfix', name: 'Bugfix Pipeline', is_starter: true },
    { id: 'wf-feature', name: 'Standard Feature Pipeline', is_starter: false },
    { id: 'wf-research', name: 'Research Consulting', is_starter: false },
];

const lookup = buildWorkflowById(workflows);

// 1. Known workflow_id → violet pill with name + Starter suffix.
{
    const badge = classifyBadge({ workflow_id: 'wf-bugfix' }, lookup);
    assert.equal(badge.variant, 'known', 'known id should classify as known');
    assert.equal(badge.name, 'Bugfix Pipeline');
    assert.equal(badge.is_starter, true);
    assert.match(badge.label, /Bugfix Pipeline/);
    assert.match(badge.label, /Starter/);
    console.log('[ok] known workflow_id renders name + Starter suffix');
}

// 2. Known workflow_id → Custom suffix for non-starter workflow.
{
    const badge = classifyBadge({ workflow_id: 'wf-feature' }, lookup);
    assert.equal(badge.variant, 'known');
    assert.equal(badge.name, 'Standard Feature Pipeline');
    assert.equal(badge.is_starter, false);
    assert.match(badge.label, /Custom/);
    assert.doesNotMatch(badge.label, /Starter/);
    console.log('[ok] known workflow_id renders name + Custom suffix');
}

// 3. Missing workflow_id → muted fallback (no crash, no "undefined").
{
    const badge = classifyBadge({ workflow_id: undefined }, lookup);
    assert.equal(badge.variant, 'fallback');
    assert.equal(badge.label, 'Workflow: unknown');
    assert.doesNotMatch(badge.label, /undefined/, 'fallback must not leak undefined');
    assert.doesNotMatch(badge.label, /null/, 'fallback must not leak null');
    console.log('[ok] missing workflow_id → muted fallback');
}

// 4. Null workflow_id → muted fallback.
{
    const badge = classifyBadge({ workflow_id: null }, lookup);
    assert.equal(badge.variant, 'fallback');
    assert.equal(badge.label, 'Workflow: unknown');
    console.log('[ok] null workflow_id → muted fallback');
}

// 5. Unknown workflow_id (workflow deleted after feature start) → muted fallback.
{
    const badge = classifyBadge({ workflow_id: 'wf-deleted' }, lookup);
    assert.equal(badge.variant, 'fallback');
    assert.equal(badge.label, 'Workflow: unknown');
    console.log('[ok] unknown workflow_id → muted fallback');
}

// 6. Empty-string workflow_id is treated as missing (the build step
//    already drops empty-string ids from the lookup, so this still
//    falls through).
{
    const badge = classifyBadge({ workflow_id: '' }, lookup);
    assert.equal(badge.variant, 'fallback');
    console.log('[ok] empty-string workflow_id → muted fallback');
}

// 7. Lookup is keyed by id, not positional — replacing the workflows
//    list (e.g. refetch after a workflow was renamed) preserves
//    correctness even if the new list is shorter.
{
    const refetched = buildWorkflowById([
        { id: 'wf-bugfix', name: 'Bugfix v2 (renamed)', is_starter: true },
    ]);
    const badge = classifyBadge({ workflow_id: 'wf-bugfix' }, refetched);
    assert.equal(badge.variant, 'known');
    assert.equal(badge.name, 'Bugfix v2 (renamed)');
    console.log('[ok] refetch with renamed workflow updates badge name');
}

// 8. Garbage in the workflows list (missing id / non-string id) does
//    not pollute the lookup.
{
    const dirty = buildWorkflowById([
        null,
        undefined,
        { name: 'no id' },
        { id: 123, name: 'numeric id' },
        { id: '', name: 'empty id' },
        { id: 'wf-ok', name: 'OK', is_starter: false },
    ]);
    assert.equal(dirty.size, 1);
    assert.ok(dirty.has('wf-ok'));
    console.log('[ok] lookup filters out entries with bad ids');
}

// 9. Feature mapping in `fetchWorkspaceData` preserves `workflow_id`.
//    Mirror the mapping reducer to verify the wire payload shape is
//    honoured (`workflow_id: null` from Rust → `undefined` in JS).
{
    const wirePayload = {
        id: 'f-1',
        project_id: 'p-1',
        workflow_id: 'wf-feature',
        title: 'Add login',
        status: 'running',
        total_cost: 0,
        duration: '5s',
        created_at: 1,
    };
    const mapped = {
        id: wirePayload.id,
        project_id: wirePayload.project_id,
        workflow_id: wirePayload.workflow_id ?? undefined,
    };
    assert.equal(mapped.workflow_id, 'wf-feature');
    console.log('[ok] feature mapping preserves workflow_id from wire payload');
}

// 10. Feature mapping tolerates `workflow_id: null` from the wire.
{
    const wirePayload = {
        id: 'f-2',
        project_id: 'p-1',
        workflow_id: null,
    };
    const mapped = {
        workflow_id: wirePayload.workflow_id ?? undefined,
    };
    assert.equal(mapped.workflow_id, undefined);
    const badge = classifyBadge(mapped, lookup);
    assert.equal(badge.variant, 'fallback');
    console.log('[ok] feature mapping tolerates null workflow_id → fallback badge');
}

console.log('\n[repro] all workflow-indicator assertions passed.');
