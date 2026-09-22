import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { McpConnectAgentPanel } from './McpConnectAgentPanel';

describe('McpConnectAgentPanel', () => {
  it('templates the live server URL into the claude-code command', () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    expect(
      screen.getByText('claude mcp add --transport http demeteo http://127.0.0.1:9999 --scope user'),
    ).toBeInTheDocument();
  });

  it('shows the honest-degrade notice, not fabricated steps, for an unverified agent', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /opencode/i }));

    expect(screen.getByText(/doesn't have confirmed setup steps for OpenCode/i)).toBeInTheDocument();
    expect(screen.getByText('http://127.0.0.1:9999')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /install skill/i })).not.toBeInTheDocument();
  });

  it('switches back to the verified claude-code steps', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /opencode/i }));
    await userEvent.click(screen.getByRole('tab', { name: /claude/i }));

    expect(screen.getByRole('button', { name: /install skill/i })).toBeInTheDocument();
  });
});
