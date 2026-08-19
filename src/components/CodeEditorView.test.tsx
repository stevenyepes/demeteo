/**
 * The far end of the review route. `reviewActions` in `src/lib/syncPanel.ts`
 * decides the refs and `useSyncActions.test.tsx` asserts the pair it hands to
 * `openDiffRange`; both stop there, so everything between that call and the
 * diff on screen was untested: replacing `head_before..merge_commit_sha` with
 * the branch pair — which shows a diff that omits the merge under review —
 * left tsc and the whole suite green.
 *
 * The two display strings are asserted as well as the refs, because they went
 * wrong in exactly the way a green suite cannot see. The content is the
 * `head_before → merge_commit` diff, which is mostly upstream work flowing
 * *in*; a header reading `master → feature/f-1` tells the reviewer an incoming
 * hunk is something the feature added, and "No changes vs master" is a flat
 * falsehood in the one screen state a reviewer most needs to trust.
 */
import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChangedFile } from '../lib/files';

const gitChangedFiles = vi.fn<(input: unknown) => Promise<ChangedFile[]>>();
const listDir = vi.fn(async () => []);

vi.mock('../lib/files', () => ({
  gitChangedFiles: (input: unknown) => gitChangedFiles(input),
  gitFileAtRef: async () => {
    throw new Error('no file was opened in these tests');
  },
  listDir: () => listDir(),
  readFile: async () => {
    throw new Error('no file was read in these tests');
  },
}));

import { CodeEditorView } from './CodeEditorView';

function mount(over: Record<string, unknown> = {}) {
  return render(
    <CodeEditorView
      machineId="local"
      worktreePath="/repos/demeteo"
      branch="feature/f-1"
      defaultBranch="master"
      featureTitle="Add a metric strip"
      onBack={() => {}}
      {...over}
    />,
  );
}

describe('CodeEditorView', () => {
  beforeEach(() => {
    gitChangedFiles.mockReset();
    gitChangedFiles.mockResolvedValue([]);
  });

  it('diffs the ref pair it was given and names it', async () => {
    mount({ baseRef: 'aaaaaaa1111', headRef: 'c0ffeec2222', initialTab: 'changes' });

    await waitFor(() =>
      expect(gitChangedFiles).toHaveBeenCalledWith({
        machineId: 'local',
        worktreePath: '/repos/demeteo',
        baseRef: 'aaaaaaa1111',
        headRef: 'c0ffeec2222',
      }),
    );
    expect(await screen.findByText('No changes vs aaaaaaa')).toBeInTheDocument();
  });

  /** Every caller but the sync review wants the branch pair, and a branch name
   *  is a label already — only a sha gets shortened. */
  it('falls back to the branch pair when no refs are given', async () => {
    mount({ initialTab: 'changes' });

    await waitFor(() =>
      expect(gitChangedFiles).toHaveBeenCalledWith({
        machineId: 'local',
        worktreePath: '/repos/demeteo',
        baseRef: 'master',
        headRef: 'feature/f-1',
      }),
    );
    expect(await screen.findByText('No changes vs master')).toBeInTheDocument();
  });

  /** The review opens on the diff, not on the file tree it would otherwise
   *  land in — a reviewer sent to a tree has to find the tab themselves. */
  it('opens on the tab it was asked for', async () => {
    mount({ baseRef: 'aaaaaaa1111', headRef: 'c0ffeec2222', initialTab: 'changes' });
    await waitFor(() => expect(gitChangedFiles).toHaveBeenCalled());

    gitChangedFiles.mockClear();
    mount();
    expect(gitChangedFiles).not.toHaveBeenCalled();
  });
});
