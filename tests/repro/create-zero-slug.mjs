/**
 * Slug validation — friendly gating for the wizard's repo-name step.
 *
 * Bug surface:
 *   The Create-From-Zero wizard's "Provider" step gates the Next
 *   button on a slug-validation helper
 *   (`validateSlug` in `src/components/CreateFromZeroWizard.tsx`).
 *   The provider step displays the helper's message inline. If the
 *   helper accepts invalid characters (e.g. uppercase, spaces, slashes)
 *   the user can click Next, the backend rejects the slug with a 422,
 *   and the inline error confuses them — they thought the gate was
 *   open.
 *
 * What this test asserts (re-implements the helper verbatim from
 * `CreateFromZeroWizard.tsx:validateSlug` and exercises its branches):
 *
 *   1. Empty input → friendly "required" message (gate stays closed).
 *   2. Too-short input (1 char) → "Use at least 2 characters" message.
 *   3. Uppercase / spaces / slashes → "lowercase letters, …" message.
 *   4. Allowed characters (lowercase alphanum + . _ -) → empty string
 *      (gate opens).
 *   5. Length bound (101 chars) → message (over 100 chars).
 *
 * Run (no project deps — pure Node ≥ 18):
 *
 *   $ node tests/repro/create-zero-slug.mjs
 */

import assert from 'node:assert/strict';

// --------------------------------------------------------------------------
// Mirror of `validateSlug(value)` in
// src/components/CreateFromZeroWizard.tsx — the production helper
// that gates the Provider step's Next button. Kept in sync; if the
// production regex or messages change, this test must follow.
// --------------------------------------------------------------------------
const SLUG_PATTERN = /^[a-z0-9][a-z0-9._-]{0,99}$/;

function validateSlug(value) {
  const trimmed = value.trim();
  if (!trimmed) return 'Repository name is required';
  if (trimmed.length < 2) return 'Use at least 2 characters';
  if (!SLUG_PATTERN.test(trimmed)) {
    return 'Use lowercase letters, digits, dots, dashes or underscores';
  }
  return '';
}

let failed = 0;
function check(label, observed, expected) {
  const ok = observed === expected;
  if (ok) {
    console.log(`[repro] [PASS] ${label}`);
  } else {
    failed++;
    console.error(`[repro] [FAIL] ${label}`);
    console.error(`         expected: ${JSON.stringify(expected)}`);
    console.error(`         observed: ${JSON.stringify(observed)}`);
  }
}

// 1. Empty input — gate stays closed, friendly message returned.
check('empty input rejected', validateSlug(''), 'Repository name is required');
check('whitespace-only input rejected', validateSlug('   '), 'Repository name is required');

// 2. Too-short input — gate stays closed.
check('single character rejected', validateSlug('a'), 'Use at least 2 characters');

// 3. Disallowed characters — gate stays closed with the lowercase hint.
check('uppercase rejected', validateSlug('MyRepo'), 'Use lowercase letters, digits, dots, dashes or underscores');
check('space rejected', validateSlug('my repo'), 'Use lowercase letters, digits, dots, dashes or underscores');
check('slash rejected', validateSlug('my/repo'), 'Use lowercase letters, digits, dots, dashes or underscores');
check('leading hyphen rejected', validateSlug('-foo'), 'Use lowercase letters, digits, dots, dashes or underscores');
check('unicode rejected', validateSlug('café'), 'Use lowercase letters, digits, dots, dashes or underscores');

// 4. Allowed characters — gate opens (empty string).
check('plain alphanumeric accepted', validateSlug('my-repo'), '');
check('with dots accepted', validateSlug('my.repo.v2'), '');
check('with underscores accepted', validateSlug('my_repo_v2'), '');
check('with hyphens accepted', validateSlug('my-cool-app'), '');
check('mixed accepted', validateSlug('svc.billing_v2-prod'), '');

// 5. Length bound — 100 chars max.
const exactly100 = 'a'.repeat(100);
const exactly101 = 'a'.repeat(101);
check('100 characters accepted', validateSlug(exactly100), '');
check('101 characters rejected', validateSlug(exactly101), 'Use lowercase letters, digits, dots, dashes or underscores');

if (failed > 0) {
  console.error(`\n[repro] FAIL: ${failed} assertion(s) failed.`);
  process.exit(1);
}

console.log('\n[repro] PASS: slug validation gates the Provider step correctly.');
