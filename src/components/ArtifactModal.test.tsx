// The claim this file defends: the artifact preview is an overlay that can be
// opened, read, and dismissed — and that costs nothing while it is closed or
// while its parent is merely re-rendering.
//
// Three failure modes are pinned here, each of which shipped once already in a
// neighbouring surface:
//
//   1. The header must name the artifact using the *shared* classifier in
//      `lib/artifacts` (basename + kind label), so this surface cannot drift
//      from `NodePanel`'s. A third copy of the classifier is what these
//      assertions exist to make visible.
//
//   2. Dismissal works from both the close button and `Escape`. Escape is
//      `ui/Modal`'s window-level `keydown`; hand-rolling the overlay (and thus
//      losing it) is the mistake `OverlayPortal`'s doc comment records.
//
//   3. Nothing is in the document while the modal is unmounted, and a parent
//      re-render does not re-render the viewer subtree. `FeatureDetail` polls
//      every 3s: an inline arrow callback or a recomputed `contentVersion`
//      handed to the memoized `ArtifactViewer` means a re-fetch of
//      `artifact_body` and a Monaco remount on every tick. Mirrors
//      `ArtifactViewer.rerender.test.tsx`, which counts renders of the real
//      markdown subtree rather than asserting that `React.memo` works.

import { invoke } from '@tauri-apps/api/core';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useCallback, useState, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ArtifactModal } from './ArtifactModal';

let markdownRenders = 0;

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => {
    markdownRenders += 1;
    return <div data-testid="markdown-body">{children}</div>;
  },
}));

const REPORT = '/tmp/artifacts/research-report.md';

beforeEach(() => {
  markdownRenders = 0;
  // A double that answers only what it was told to answer: any other command
  // is a bug in the component, not a case to silently pass.
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    if (cmd === 'artifact_body') {
      return Promise.resolve('# Research report\n\nSome body text.');
    }
    return Promise.reject(new Error(`unexpected IPC command: ${cmd}`));
  }) as unknown as typeof invoke);
});

describe('ArtifactModal', () => {
  it('renders the basename, the shared kind label and the humanized step', async () => {
    render(
      <ArtifactModal artifactPath={REPORT} stepId="s-code-review" onClose={() => {}} />,
    );

    // Anchored: `toHaveTextContent` is a substring match, so the unanchored
    // form passes just as happily when the header renders the whole
    // `/tmp/artifacts/...` path — which is the regression this case exists for.
    expect(screen.getByTestId('artifact-modal-title')).toHaveTextContent(
      /^research-report\.md$/,
    );
    expect(screen.getByText('Markdown')).toBeInTheDocument();
    expect(screen.getByText('· Code Review')).toBeInTheDocument();
    await screen.findByTestId('markdown-body');
  });

  it('closes on the close button', async () => {
    const onClose = vi.fn();
    render(<ArtifactModal artifactPath={REPORT} onClose={onClose} />);

    await userEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(onClose).toHaveBeenCalledTimes(1);
    await screen.findByTestId('markdown-body');
  });

  it('closes on Escape', async () => {
    const onClose = vi.fn();
    render(<ArtifactModal artifactPath={REPORT} onClose={onClose} />);

    fireEvent.keyDown(window, { key: 'Escape' });

    expect(onClose).toHaveBeenCalledTimes(1);
    await screen.findByTestId('markdown-body');
  });

  it('puts nothing in the document while no artifact is selected', async () => {
    // The mount-only-while-open contract: with no selection there is no
    // header, no viewer, and — the part that matters on a 3s poll — no
    // `artifact_body` fetch and no Monaco/ReactMarkdown construction.
    function RunColumnToggle() {
      const [path, setPath] = useState<string | null>(null);

      return (
        <div>
          <button onClick={() => setPath(REPORT)}>select artifact</button>
          {path && <ArtifactModal artifactPath={path} onClose={() => setPath(null)} />}
        </div>
      );
    }

    render(<RunColumnToggle />);

    expect(screen.queryByTestId('artifact-modal-title')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Close' })).not.toBeInTheDocument();
    expect(screen.queryByTestId('markdown-body')).not.toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole('button', { name: 'select artifact' }));

    expect(await screen.findByTestId('markdown-body')).toBeInTheDocument();
    expect(screen.getByTestId('artifact-modal-title')).toHaveTextContent(
      /^research-report\.md$/,
    );
  });

  it('does not re-render the viewer subtree when its parent re-renders', async () => {
    // Mirrors FeatureDetail: the parent owns unrelated state (there, a 3s poll)
    // and a `useCallback`-stable editor handler.
    function RunColumnShape() {
      const [note, setNote] = useState('');
      const openEditor = useCallback((_filePath: string) => {}, []);

      return (
        <div>
          <textarea aria-label="note" value={note} onChange={(e) => setNote(e.target.value)} />
          <ArtifactModal
            artifactPath={REPORT}
            stepId="s-implement"
            contentVersion="completed:120:8:0.01"
            onClose={() => {}}
            onOpenEditorForPath={openEditor}
          />
        </div>
      );
    }

    render(<RunColumnShape />);

    await waitFor(() => {
      expect(screen.getByTestId('markdown-body')).toBeInTheDocument();
    });
    const rendersAfterLoad = markdownRenders;

    await userEvent.type(screen.getByLabelText('note'), 'hello');

    expect(screen.getByLabelText('note')).toHaveValue('hello');
    expect(markdownRenders).toBe(rendersAfterLoad);
    // One fetch for the artifact, not one per parent render.
    expect(vi.mocked(invoke).mock.calls.filter(([cmd]) => cmd === 'artifact_body')).toHaveLength(1);
  });

  it('classifies a path with no extension', async () => {
    render(<ArtifactModal artifactPath="/tmp/artifacts/NOTES" onClose={() => {}} />);

    expect(screen.getByTestId('artifact-modal-title')).toHaveTextContent('NOTES');
    expect(screen.getByText('Text')).toBeInTheDocument();
    // No markdown for an extension-less file — the code viewer takes it.
    expect(await screen.findByTestId('monaco-editor')).toBeInTheDocument();
  });

  it('truncates a very long basename instead of overflowing', async () => {
    const long = `/tmp/artifacts/${'a-very-long-artifact-name-'.repeat(8)}final.md`;
    render(<ArtifactModal artifactPath={long} onClose={() => {}} />);

    const title = screen.getByTestId('artifact-modal-title');
    expect(title.className).toContain('truncate');
    expect(title).toHaveAttribute('title', long.split('/').pop());
    await screen.findByTestId('markdown-body');
  });

  it('omits the editor action when no handler is given', async () => {
    render(<ArtifactModal artifactPath={REPORT} onClose={() => {}} />);

    expect(screen.queryByRole('button', { name: /open in editor/i })).not.toBeInTheDocument();
    await screen.findByTestId('markdown-body');
  });

  it('opens the editor with the artifact path when a handler is given', async () => {
    const onOpenEditorForPath = vi.fn();
    render(
      <ArtifactModal
        artifactPath={REPORT}
        onClose={() => {}}
        onOpenEditorForPath={onOpenEditorForPath}
      />,
    );

    await userEvent.click(screen.getByRole('button', { name: /open in editor/i }));

    expect(onOpenEditorForPath).toHaveBeenCalledWith(REPORT);
    await screen.findByTestId('markdown-body');
  });
});
