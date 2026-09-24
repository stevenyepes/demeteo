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

  it('shows an entry\'s own caveat when its evidence is weaker than a live run', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /hermes/i }));

    expect(screen.getByText(/not yet run against Demeteo/i)).toBeInTheDocument();
    expect(screen.queryByText(/Checked against Demeteo up to the approval prompt/i)).not.toBeInTheDocument();
    expect(
      screen.getByText('hermes mcp add demeteo --url http://127.0.0.1:9999 --auth oauth'),
    ).toBeInTheDocument();
  });

  it('templates the URL into an unconfirmed agent\'s steps under a caveat', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /codex/i }));

    expect(screen.getByText(/a full Codex sign-in hasn't been confirmed yet/i)).toBeInTheDocument();
    expect(screen.getByText('codex mcp add demeteo --url http://127.0.0.1:9999')).toBeInTheDocument();
  });

  it('keeps the explicit oauth flag in Pi\'s config snippet', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /^pi$/i }));

    expect(
      screen.getByText('{ "mcpServers": { "demeteo": { "url": "http://127.0.0.1:9999", "auth": "oauth" } } }'),
    ).toBeInTheDocument();
  });

  it('switches back to the verified claude-code steps', async () => {
    render(<McpConnectAgentPanel serverUrl="http://127.0.0.1:9999" />);

    await userEvent.click(screen.getByRole('tab', { name: /opencode/i }));
    await userEvent.click(screen.getByRole('tab', { name: /claude/i }));

    expect(screen.getByRole('button', { name: /install skill/i })).toBeInTheDocument();
  });
});
