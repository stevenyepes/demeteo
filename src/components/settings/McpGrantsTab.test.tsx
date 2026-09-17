// implementation-spec.md AC7 first half: a vitest test renders the grants tab
// from a fixture list (client name, scopes, expiry) and asserts the revoke
// control calls the typed `revokeMcpGrant` wrapper with the clicked row's id.
//
// `invoke` is mocked globally in `src/test/setup.ts`; this suite scripts it
// directly per command, the same idiom as `AddressFindingsLaunch.test.tsx`.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

const reportError = vi.fn();
vi.mock('../../lib/errorBus', () => ({
  reportError: (...args: unknown[]) => reportError(...args),
}));

import { McpGrantsTab } from './McpGrantsTab';

const mockedInvoke = vi.mocked(invoke);

afterEach(() => {
  reportError.mockReset();
});

function grant(over: Record<string, unknown> = {}) {
  return {
    id: 'grant-1',
    client_name: 'Claude Desktop',
    scopes: ['read', 'spend'],
    issued_at: Date.parse('2026-08-01T00:00:00Z'),
    expires_at: Date.parse('2026-09-30T00:00:00Z'),
    revoked: false,
    ...over,
  };
}

const DEFAULT_SERVER_STATUS = { enabled: false, url: null as string | null };

function backend(
  grants: Record<string, unknown>[],
  serverStatus: { enabled: boolean; url: string | null } = DEFAULT_SERVER_STATUS,
  /** The `{ enabled, url }` `set_mcp_server_enabled` should report on its
   *  post-toggle `get_mcp_server_status` re-fetch when toggled on — lets a
   *  test express a bind failure (`enabled: true, url: null`) that a fixed
   *  `enabled ? 'http://...' : null` formula could never produce. */
  enabledStatus: { enabled: boolean; url: string | null } = {
    enabled: true,
    url: 'http://127.0.0.1:8765',
  },
) {
  const revoked: string[] = [];
  let server = { ...serverStatus };
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    if (cmd === 'list_mcp_grants') return grants;
    if (cmd === 'revoke_mcp_grant') {
      const { grantId } = (args ?? {}) as { grantId?: string };
      if (grantId) revoked.push(grantId);
      return undefined;
    }
    if (cmd === 'get_mcp_server_status') return server;
    if (cmd === 'set_mcp_server_enabled') {
      const { enabled } = (args ?? {}) as { enabled?: boolean };
      server = enabled ? { ...enabledStatus } : { enabled: false, url: null };
      return undefined;
    }
    throw new Error(`unscripted invoke('${cmd}')`);
  }) as typeof invoke);
  return revoked;
}

describe('McpGrantsTab', () => {
  it('renders client name, scopes, and expiry for each grant', async () => {
    backend([
      grant(),
      grant({ id: 'grant-2', client_name: 'Cursor', scopes: ['read'] }),
    ]);

    render(<McpGrantsTab />);

    expect(await screen.findByText('Claude Desktop')).toBeInTheDocument();
    expect(screen.getByText('Cursor')).toBeInTheDocument();
    expect(screen.getAllByText('read')).toHaveLength(2);
    expect(screen.getByText('spend')).toBeInTheDocument();
    expect(
      screen.getAllByText(`Expires ${new Date(grant().expires_at).toLocaleString()}`),
    ).toHaveLength(2);
  });

  it('calls revokeMcpGrant with the clicked row\'s id and removes that row', async () => {
    backend([
      grant(),
      grant({ id: 'grant-2', client_name: 'Cursor', scopes: ['read'] }),
    ]);

    render(<McpGrantsTab />);
    await screen.findByText('Claude Desktop');

    const rows = screen.getAllByRole('button', { name: /revoke/i });
    await userEvent.click(rows[0]);

    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith('revoke_mcp_grant', { grantId: 'grant-1' }),
    );
    await waitFor(() => expect(screen.queryByText('Claude Desktop')).not.toBeInTheDocument());
    expect(screen.getByText('Cursor')).toBeInTheDocument();
  });

  it('shows an empty state when there are no active grants', async () => {
    backend([]);

    render(<McpGrantsTab />);

    expect(await screen.findByText(/no active mcp client grants/i)).toBeInTheDocument();
  });
});

describe('McpGrantsTab MCP server toggle', () => {
  it('reads status on mount and shows Stopped with no URL when disabled', async () => {
    backend([], { enabled: false, url: null });

    render(<McpGrantsTab />);

    const toggle = await screen.findByRole('switch', { name: /enable mcp server/i });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByText('Stopped')).toBeInTheDocument();
    expect(screen.queryByText(/^http:\/\//)).not.toBeInTheDocument();
    expect(mockedInvoke).toHaveBeenCalledWith('get_mcp_server_status');
  });

  it('shows Listening and the endpoint URL when the server starts enabled', async () => {
    backend([], { enabled: true, url: 'http://127.0.0.1:8765' });

    render(<McpGrantsTab />);

    expect(await screen.findByText('Listening')).toBeInTheDocument();
    expect(await screen.findByText('http://127.0.0.1:8765')).toBeInTheDocument();
  });

  it('flips the toggle optimistically and calls setMcpServerEnabled with exactly { enabled: true }', async () => {
    backend([], { enabled: false, url: null });

    render(<McpGrantsTab />);
    const toggle = await screen.findByRole('switch', { name: /enable mcp server/i });

    await userEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-checked', 'true');
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith('set_mcp_server_enabled', { enabled: true }),
    );
    expect(await screen.findByText('http://127.0.0.1:8765')).toBeInTheDocument();
  });

  it('rolls back the toggle and surfaces the error when the invoke call rejects', async () => {
    mockedInvoke.mockImplementation((async (cmd: string) => {
      if (cmd === 'list_mcp_grants') return [];
      if (cmd === 'get_mcp_server_status') return { enabled: false, url: null };
      if (cmd === 'set_mcp_server_enabled') throw new Error('bind failed');
      throw new Error(`unscripted invoke('${cmd}')`);
    }) as typeof invoke);

    render(<McpGrantsTab />);
    const toggle = await screen.findByRole('switch', { name: /enable mcp server/i });

    await userEvent.click(toggle);

    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
    expect(mockedInvoke).toHaveBeenCalledWith('set_mcp_server_enabled', { enabled: true });
    expect(screen.queryByText(/^http:\/\//)).not.toBeInTheDocument();
  });

  it('shows a distinct not-listening state and surfaces an error on initial mount when enabled with nothing bound', async () => {
    backend([], { enabled: true, url: null });

    const { container } = render(<McpGrantsTab />);

    expect(await screen.findByText('Not Listening')).toBeInTheDocument();
    expect(screen.queryByText('Listening')).not.toBeInTheDocument();
    expect(screen.queryByText(/^http:\/\//)).not.toBeInTheDocument();
    expect(container.querySelector('.bg-emerald-400')).not.toBeInTheDocument();
    expect(container.querySelector('.bg-amber-400')).toBeInTheDocument();
    expect(await screen.findByRole('alert')).toHaveTextContent(/enabled, but not listening/i);
    expect(screen.getByRole('status')).toHaveTextContent('Not Listening');
    await waitFor(() => expect(reportError).toHaveBeenCalled());
  });

  it('shows a distinct not-listening state and surfaces an error when set_mcp_server_enabled resolves but the bind failed', async () => {
    backend([], { enabled: false, url: null }, { enabled: true, url: null });

    const { container } = render(<McpGrantsTab />);
    const toggle = await screen.findByRole('switch', { name: /enable mcp server/i });

    await userEvent.click(toggle);

    expect(await screen.findByText('Not Listening')).toBeInTheDocument();
    expect(screen.queryByText('Listening')).not.toBeInTheDocument();
    expect(screen.queryByText(/^http:\/\//)).not.toBeInTheDocument();
    expect(container.querySelector('.bg-emerald-400')).not.toBeInTheDocument();
    expect(container.querySelector('.bg-amber-400')).toBeInTheDocument();
    expect(toggle).toHaveAttribute('aria-checked', 'true');
    expect(mockedInvoke).toHaveBeenCalledWith('set_mcp_server_enabled', { enabled: true });
    expect(await screen.findByRole('alert')).toHaveTextContent(/enabled, but not listening/i);
    expect(screen.getByRole('status')).toHaveTextContent('Not Listening');
    await waitFor(() => expect(reportError).toHaveBeenCalled());
  });
});
