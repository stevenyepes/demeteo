// Type-level smoke tests for the AppView discriminated union in `src/types.ts`.
//
// Deliberately NOT a `.test.ts`: there is nothing to execute here, so it is
// excluded from the Vitest glob. `tsc --noEmit` is the real gate — it exercises
// the wiring of the `create-project` variant by producing an `AppView` of that
// kind and guaranteeing the existing `kind: 'new-project'` / `kind: 'home'`
// shapes remain assignable. Remove the variant from the union and this file
// fails to type-check.

import type { AppView } from './types';

function asCreateProject(): AppView {
  return { kind: 'create-project' };
}

const emptyState: AppView = { kind: 'empty-state' };
const home: AppView = { kind: 'home' };
const newProject: AppView = { kind: 'new-project' };
const createProject: AppView = asCreateProject();
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
  createProject,
  detail,
  editor,
} as const;

export type AppViewKind = AppView['kind'];
