// Regression: typing in the gate feedback textarea must not re-render the
// artifact viewer.
//
// The bug: in GateView the feedback <textarea> and <ArtifactViewer> share a
// parent that holds `feedback` in state. Every keystroke re-rendered the
// parent, and with it the heavy ReactMarkdown subtree (and its embedded Monaco
// editors). Users saw "the markdown view refreshing every time I type a key".
//
// The fix was `export const ArtifactViewer = memo(ArtifactViewerInner)` plus a
// stable `artifactPath` prop.
//
// Was `tests/repro/gate-feedback-rerender.mjs`, which built its own
// memo-wrapped stand-in component and asserted that React.memo works —
// tautological, and it would still have passed if someone unwrapped the real
// ArtifactViewer. This mounts the real component and counts renders of the real
// markdown subtree by way of a counting react-markdown stub.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState, type ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ArtifactViewer } from './ArtifactViewer';

let markdownRenders = 0;

vi.mock('react-markdown', () => ({
  default: ({ children }: { children?: ReactNode }) => {
    markdownRenders += 1;
    return <div data-testid="markdown-body">{children}</div>;
  },
}));

// Mirrors GateView: one parent owning the feedback state, with the heavy
// viewer rendered as a sibling of the textarea.
function GateViewShape() {
  const [feedback, setFeedback] = useState('');

  return (
    <div>
      <ArtifactViewer artifactPath="/tmp/research-report.md" maxHeight="280px" />
      <textarea
        aria-label="feedback"
        value={feedback}
        onChange={(e) => setFeedback(e.target.value)}
      />
    </div>
  );
}

beforeEach(() => {
  markdownRenders = 0;
  vi.mocked(invoke).mockResolvedValue('# Research report\n\nSome body text.');
});

describe('ArtifactViewer inside the gate feedback loop', () => {
  it('does not re-render the markdown subtree on every keystroke', async () => {
    render(<GateViewShape />);

    await waitFor(() => {
      expect(screen.getByTestId('markdown-body')).toBeInTheDocument();
    });

    const rendersAfterLoad = markdownRenders;

    await userEvent.type(screen.getByLabelText('feedback'), 'hello');

    expect(screen.getByLabelText('feedback')).toHaveValue('hello');
    // Five keystrokes, zero extra markdown renders. Without the memo this
    // climbs by one per character.
    expect(markdownRenders).toBe(rendersAfterLoad);
  });

  it('keeps ArtifactViewer memoized', () => {
    // Guards the fix directly: unwrapping `memo(...)` fails here even if the
    // render-count test above is somehow satisfied another way.
    expect(ArtifactViewer).toHaveProperty('$$typeof', Symbol.for('react.memo'));
  });
});
