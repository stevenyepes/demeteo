/**
 * The list is the one place a user is told which files the merge left, and its
 * rows are the one way into them. Both halves went untested, and the second was
 * wrong for as long as it existed: every row routed through the *feature*
 * worktree, so clicking a conflicted path opened a clean, marker-free copy of
 * that same path with nothing on screen to say it was a different file.
 *
 * What that miss needs is a test over the row's contract — it hands the path
 * back, and nothing more — with `FeatureDetail.sync.test.tsx` asserting which
 * checkout the path is then resolved in.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import type { ConflictFile } from '../../../types';
import { ConflictFileList } from './ConflictFileList';

const files: ConflictFile[] = [
  { path: 'src/components/ask/AskCanvasPane.test.tsx', kind: 'both-modified' },
  { path: 'crates/demeteo-core/src/application/ask/mod.rs', kind: 'added-by-them' },
];

describe('ConflictFileList', () => {
  it('names every unmerged path and how git classified it', () => {
    render(<ConflictFileList files={files} onOpenPath={vi.fn()} />);

    const rows = screen.getAllByRole('button');
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('src/components/ask/AskCanvasPane.test.tsx');
    expect(rows[0]).toHaveTextContent('both-modified');
    expect(rows[1]).toHaveTextContent('added-by-them');
  });

  it('hands back the path of the row that was pressed', async () => {
    const onOpenPath = vi.fn();
    render(<ConflictFileList files={files} onOpenPath={onOpenPath} />);

    await userEvent.click(
      screen.getByRole('button', { name: /AskCanvasPane\.test\.tsx/ }),
    );

    expect(onOpenPath).toHaveBeenCalledTimes(1);
    expect(onOpenPath).toHaveBeenCalledWith('src/components/ask/AskCanvasPane.test.tsx');
  });

  /** An empty list is a failed read, not a small conflict — the porcelain that
   *  fills it answers empty on any transport error. Saying so is the whole
   *  reason this case renders a sentence instead of nothing. */
  it('says an empty list is a read that failed', () => {
    render(<ConflictFileList files={[]} onOpenPath={vi.fn()} />);

    expect(screen.getByText(/read that failed/)).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });
});
