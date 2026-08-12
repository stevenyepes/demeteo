// Unit tests for `src/lib/artifacts.tsx`.

import { describe, it, expect } from 'vitest';
import { classifyArtifact, ARTIFACT_KIND_LABELS, ARTIFACT_KIND_COLORS } from './artifacts';

describe('classifyArtifact', () => {
  it('classifies a top-level task-list.json as task-list', () => {
    expect(classifyArtifact('artifacts/task-list.json')).toEqual({
      kind: 'task-list',
      ext: 'json',
      basename: 'task-list.json',
    });
  });

  it('classifies a nested task-list.json as task-list', () => {
    expect(classifyArtifact('some/nested/dir/task-list.json')).toEqual({
      kind: 'task-list',
      ext: 'json',
      basename: 'task-list.json',
    });
  });

  it('classifies task-list.json case-insensitively, like every other branch', () => {
    expect(classifyArtifact('artifacts/Task-List.JSON').kind).toBe('task-list');
  });

  it('does not classify a merely task-list-ish name as a task list', () => {
    expect(classifyArtifact('artifacts/task-list-schema.json').kind).toBe('json');
  });

  it('classifies other .json files as json', () => {
    expect(classifyArtifact('artifacts/validation-report.json')).toEqual({
      kind: 'json',
      ext: 'json',
      basename: 'validation-report.json',
    });
  });

  it('still classifies *.worktree-ref.json as json, per the documented quirk (regression)', () => {
    // The module docstring documents this as deliberate and unreachable: a
    // `.worktree-ref.json` path also ends in `.json`, so the earlier generic
    // check wins. `task-list.json` classification must not disturb it.
    expect(classifyArtifact('artifacts/foo.worktree-ref.json')).toEqual({
      kind: 'json',
      ext: 'json',
      basename: 'foo.worktree-ref.json',
    });
  });
});

describe('ARTIFACT_KIND_LABELS / ARTIFACT_KIND_COLORS', () => {
  it('have a task-list entry', () => {
    expect(ARTIFACT_KIND_LABELS['task-list']).toBeTruthy();
    expect(ARTIFACT_KIND_COLORS['task-list']).toBeTruthy();
  });
});
