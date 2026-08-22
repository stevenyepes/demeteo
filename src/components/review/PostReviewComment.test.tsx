// The one control in the tree whose mistake is unrecoverable: a comment on
// someone else's pull request cannot be taken back from here. So the assertions
// are about restraint — that pressing the button alone posts nothing, and that
// a rejected post never renders as a posted one.

import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { PostReviewComment } from './PostReviewComment';

const REPORT = '## Findings\n\nThe refspec guard holds.';
const COMMENT_URL = 'https://github.com/acme/app/pull/412#issuecomment-1';

/** Answers `post_pull_request_comment` and rejects everything else, so a
 *  component calling a neighbouring command fails here rather than passing
 *  against a stub that says yes to anything. */
function backend(answer: () => Promise<unknown>) {
  vi.mocked(invoke).mockImplementation((cmd: string) => {
    if (cmd === 'post_pull_request_comment') return answer();
    return Promise.reject(new Error(`unexpected command: ${cmd}`));
  });
}

function mount(report = REPORT) {
  return render(
    <PostReviewComment
      projectId="proj-1"
      pullRequestUrl="https://github.com/acme/app/pull/412"
      pullRequestLabel="PR #412"
      report={report}
    />,
  );
}

describe('PostReviewComment', () => {
  it('posts nothing until the confirmation is answered', async () => {
    backend(() => Promise.resolve(COMMENT_URL));
    mount();

    await userEvent.click(screen.getByTestId('post-review-comment'));

    expect(
      screen.getByText(/visible to everyone with access to the repository/),
    ).toBeInTheDocument();
    expect(vi.mocked(invoke)).not.toHaveBeenCalled();
  });

  it('names the pull request in the confirmation', async () => {
    backend(() => Promise.resolve(COMMENT_URL));
    mount();

    await userEvent.click(screen.getByTestId('post-review-comment'));

    expect(screen.getByText(/as a comment on PR #412/)).toBeInTheDocument();
  });

  it('sends the report and reports the comment back', async () => {
    backend(() => Promise.resolve(COMMENT_URL));
    mount();

    await userEvent.click(screen.getByTestId('post-review-comment'));
    await userEvent.click(screen.getByTestId('confirm-post-review-comment'));

    await waitFor(() =>
      expect(screen.getByTestId('posted-comment-link')).toHaveAttribute('href', COMMENT_URL),
    );
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('post_pull_request_comment', {
      projectId: 'proj-1',
      pullRequestUrl: 'https://github.com/acme/app/pull/412',
      body: REPORT,
    });
  });

  it('offers no second post once one has landed', async () => {
    backend(() => Promise.resolve(COMMENT_URL));
    mount();

    await userEvent.click(screen.getByTestId('post-review-comment'));
    await userEvent.click(screen.getByTestId('confirm-post-review-comment'));

    await waitFor(() => expect(screen.getByTestId('posted-comment-link')).toBeInTheDocument());
    expect(screen.queryByTestId('post-review-comment')).not.toBeInTheDocument();
  });

  it('says a rejected post failed rather than showing it as posted', async () => {
    backend(() => Promise.reject(new Error('GitHub returned HTTP 403: not accessible')));
    mount();

    await userEvent.click(screen.getByTestId('post-review-comment'));
    await userEvent.click(screen.getByTestId('confirm-post-review-comment'));

    await waitFor(() =>
      expect(screen.getByTestId('post-review-failed')).toHaveTextContent('403'),
    );
    expect(screen.queryByTestId('posted-comment-link')).not.toBeInTheDocument();
  });

  it('cannot be pressed with no report to post', () => {
    backend(() => Promise.resolve(COMMENT_URL));
    mount('   \n');

    expect(screen.getByTestId('post-review-comment')).toBeDisabled();
  });
});
