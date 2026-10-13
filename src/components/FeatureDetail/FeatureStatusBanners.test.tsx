/**
 * What the banner strip must *not* say any more.
 *
 * Sync grew five surfaces here, one per phase, and each was correct alone. The
 * failure mode of consolidating them is the one no compiler catches: a retired
 * banner that is still rendered somewhere reads as a second, contradictory
 * answer beside the pane — so this pins the strip down to the one nudge left.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { FeatureStatusBanners } from './FeatureStatusBanners';

function mount(status = 'completed', publishError: string | null = null) {
  return render(<FeatureStatusBanners status={status} publishError={publishError} />);
}

describe('FeatureStatusBanners', () => {
  it('renders no sync surface at all', () => {
    mount();

    expect(screen.queryByTestId('sync-review')).toBeNull();
    expect(screen.queryByTestId('sync-panel')).toBeNull();
    expect(screen.queryByRole('button', { name: /resolve with agent/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /abort sync/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /discard merge/i })).toBeNull();
  });

  it('still nudges a finished run that opened no pull request', () => {
    mount('awaiting_mr');
    expect(screen.getByText(/no PR was opened/i)).toBeInTheDocument();
  });

  it('renders nothing for a finished run that has nothing to nudge about', () => {
    const { container } = mount('completed');
    expect(container).toBeEmptyDOMElement();
  });

  it("names a detached run's failed push and shows the hook's output", () => {
    mount(
      'awaiting_mr',
      'failed to push feature branch to origin:\n\u001b[1;31m✖ preflight failed\u001b[0m\n  - node_modules/.bin/tsc — run \'npm ci\'\nerror: failed to push some refs',
    );
    expect(screen.getByText(/the publish failed/i)).toBeInTheDocument();
    expect(screen.queryByText(/workflow has no finalize step/i)).toBeNull();
    const output = screen.getByText(/node_modules\/\.bin\/tsc/);
    expect(output.textContent).toContain('✖ preflight failed');
    expect(output.textContent).not.toContain('\u001b');
  });
});
