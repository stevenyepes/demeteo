// Type-level smoke tests for the AppView discriminated union in
// `src/types.ts`. These tests do NOT require a runtime test runner —
// they exist purely so that `tsc --noEmit` exercises the wiring of
// the `create-from-zero` variant: producing an `AppView` of that kind
// and guaranteeing the existing `kind: 'new-project'` / `kind: 'home'`
// shapes remain assignable. If anyone ever removes the variant from
// the union, the test file will fail to type-check.

import type { AppView } from './types';

function asCreateFromZero(): AppView {
  return { kind: 'create-from-zero' };
}

const emptyState: AppView = { kind: 'empty-state' };
const home: AppView = { kind: 'home' };
const newProject: AppView = { kind: 'new-project' };
const createFromZero: AppView = asCreateFromZero();
const detail: AppView = {
  kind: 'detail',
  featureId: 'feat-test',
  featureTitle: 'test feature',
};
const editor: AppView = {
  kind: 'editor',
  featureId: 'feat-test',
  featureTitle: 'test feature',
  editorContext: {
    machineId: 'machine-test',
    worktreePath: '/tmp/wt',
    branch: 'demeteo/features/test',
    defaultBranch: 'main',
  },
};

export const appViewVariants = {
  emptyState,
  home,
  newProject,
  createFromZero,
  detail,
  editor,
} as const;

export type AppViewKind = AppView['kind'];
