/**
 * What the banner strip must *not* say any more.
 *
 * Sync grew five surfaces here, one per phase, and each was correct alone. The
 * failure mode of consolidating them is the one no compiler catches: a retired
 * banner that is still rendered somewhere reads as a second, contradictory
 * answer beside the pane — so this pins the strip down to the pull request.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { FeatureStatusBanners } from './FeatureStatusBanners';

function mount(status = 'completed') {
  return render(
    <FeatureStatusBanners
      status={status}
      mrUrl={null}
      mrState={null}
      onRefreshMrState={() => {}}
    />,
  );
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

  it('shows the published request row', () => {
    render(
      <FeatureStatusBanners
        status="completed"
        mrUrl="https://example.test/pr/1"
        mrState="open"
        onRefreshMrState={() => {}}
      />,
    );
    expect(screen.getByRole('link', { name: /example.test/ })).toBeInTheDocument();
  });
});
