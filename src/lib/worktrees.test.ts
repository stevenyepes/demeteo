import { describe, expect, it } from 'vitest';

import { deriveWorktreeName } from './worktrees';

describe('deriveWorktreeName', () => {
  it('flattens a slash-separated branch into one directory name', () => {
    expect(deriveWorktreeName('feature/new-thing')).toBe('feature-new-thing');
    expect(deriveWorktreeName('fix/auth/token')).toBe('fix-auth-token');
  });

  it('keeps the characters a directory may carry and replaces the rest', () => {
    expect(deriveWorktreeName('release_1.2.x')).toBe('release_1.2.x');
    expect(deriveWorktreeName("o'reilly branch")).toBe('o-reilly-branch');
  });

  it('never produces a name the backend would refuse', () => {
    // Leading dots hide the directory and are refused as a branch component;
    // a trailing separator leaves a name git and the shell both read oddly.
    expect(deriveWorktreeName('../escape')).toBe('escape');
    expect(deriveWorktreeName('.hidden')).toBe('hidden');
    expect(deriveWorktreeName('trailing/')).toBe('trailing');
    expect(deriveWorktreeName('  ')).toBe('');
  });

  it('collapses runs so one typo does not become a run of separators', () => {
    expect(deriveWorktreeName('feature//deep   name')).toBe('feature-deep-name');
  });
});
