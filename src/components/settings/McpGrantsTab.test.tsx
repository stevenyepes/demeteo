// implementation-spec.md AC7 first half: a vitest test renders the grants tab
// from a fixture list (client name, scopes, expiry) and asserts the revoke
// control calls the typed `revokeMcpGrant` wrapper with the clicked row's id.
//
// `invoke` is mocked globally in `src/test/setup.ts`; this suite scripts it
// directly per command, the same idiom as `AddressFindingsLaunch.test.tsx`.

import { invoke } from '@tauri-apps/api/core';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { McpGrantsTab } from './McpGrantsTab';

const mockedInvoke = vi.mocked(invoke);

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

function backend(grants: Record<string, unknown>[]) {
  const revoked: string[] = [];
  mockedInvoke.mockImplementation((async (cmd: string, args?: unknown) => {
    if (cmd === 'list_mcp_grants') return grants;
    if (cmd === 'revoke_mcp_grant') {
      const { grantId } = (args ?? {}) as { grantId?: string };
      if (grantId) revoked.push(grantId);
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
