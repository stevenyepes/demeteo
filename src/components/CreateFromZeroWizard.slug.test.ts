// Slug validation for the Create-From-Zero wizard's Provider step.
//
// `validateSlug` gates the Next button: it returns '' when the name is usable
// and a user-facing message otherwise.
//
// Was `tests/repro/create-zero-slug.mjs`, which re-declared the regex and the
// messages inline and carried a "kept in sync by hand" warning. It now imports
// the real function, so production drift fails the test instead of passing it.

import { describe, expect, it } from 'vitest';

import { validateSlug } from './CreateFromZeroWizard';

const BAD_CHARS = 'Use lowercase letters, digits, dots, dashes or underscores';

describe('validateSlug', () => {
  it.each([
    ['', 'Repository name is required'],
    ['   ', 'Repository name is required'],
    ['a', 'Use at least 2 characters'],
    ['MyRepo', BAD_CHARS],
    ['my repo', BAD_CHARS],
    ['my/repo', BAD_CHARS],
    ['-foo', BAD_CHARS],
    ['café', BAD_CHARS],
  ])('rejects %j', (input, message) => {
    expect(validateSlug(input)).toBe(message);
  });

  it.each([
    'my-repo',
    'my.repo.v2',
    'my_repo_v2',
    'my-cool-app',
    'svc.billing_v2-prod',
  ])('accepts %j', (input) => {
    expect(validateSlug(input)).toBe('');
  });

  it('caps the name at 100 characters', () => {
    expect(validateSlug('a'.repeat(100))).toBe('');
    expect(validateSlug('a'.repeat(101))).toBe(BAD_CHARS);
  });
});
