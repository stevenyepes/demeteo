import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { GitHosts as Listed } from '../lib/gitHosts';
import { GitHosts } from './GitHosts';

const hosts = vi.hoisted(() => ({
  listGitHosts: vi.fn<() => Promise<Listed>>(),
  startManifest: vi.fn<(host: string, organization: string) => Promise<unknown>>(),
  removeGitHost: vi.fn<(id: string) => Promise<void>>(),
  submitManifest: vi.fn<(form: unknown) => void>(),
}));

vi.mock('../lib/gitHosts', () => hosts);

const CONNECTED = {
  id: 'h-1',
  host: 'github.com',
  appId: 5647010,
  appSlug: 'demeteo-hub-example',
  account: 'octo-org',
  installUrl: 'https://github.com/apps/demeteo-hub-example/installations/new',
  connectedAt: 1_762_400_000,
};

afterEach(() => {
  vi.clearAllMocks();
});

describe('GitHosts', () => {
  it('says runs wait while no App is connected', async () => {
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com'], hosts: [] });
    render(<GitHosts />);

    expect(await screen.findByText(/None yet/)).toBeInTheDocument();
    expect(screen.queryByLabelText('Host')).toBeNull();
  });

  it('sends the browser to GitHub with the manifest for the typed organization', async () => {
    const form = { actionUrl: 'https://github.com/organizations/octo-org/settings/apps/new', manifest: '{}', state: 's' };
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com'], hosts: [] });
    hosts.startManifest.mockResolvedValue(form);
    render(<GitHosts />);

    await userEvent.type(await screen.findByLabelText('Organization (optional)'), 'octo-org');
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));

    expect(hosts.startManifest).toHaveBeenCalledWith('github.com', 'octo-org');
    expect(hosts.submitManifest).toHaveBeenCalledWith(form);
  });

  it('offers a choice of host only when there is more than one', async () => {
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com', 'ghe.example'], hosts: [] });
    render(<GitHosts />);

    const select = await screen.findByLabelText('Host');
    await userEvent.selectOptions(select, 'ghe.example');
    await userEvent.click(screen.getByRole('button', { name: 'Connect GitHub' }));
    expect(hosts.startManifest).toHaveBeenCalledWith('ghe.example', '');
  });

  it('after GitHub sends the browser back, links straight to choosing repositories', async () => {
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com'], hosts: [CONNECTED] });
    render(<GitHosts connected="h-1" />);

    const banner = await screen.findByRole('region', { name: 'App connected' });
    expect(within(banner).getByRole('link', { name: 'Choose repositories on GitHub' })).toHaveAttribute(
      'href',
      CONNECTED.installUrl,
    );
  });

  it('explains a failed return in words, not its code', async () => {
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com'], hosts: [] });
    render(<GitHosts failed="import_forbidden" />);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent(/OpenBao policy/);
    expect(alert).not.toHaveTextContent('import_forbidden');
  });

  it('removes an App only once the removal is confirmed', async () => {
    hosts.listGitHosts.mockResolvedValue({ connectable: ['github.com'], hosts: [CONNECTED] });
    hosts.removeGitHost.mockResolvedValue();
    render(<GitHosts />);

    await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    expect(hosts.removeGitHost).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Remove from this Hub' }));
    expect(hosts.removeGitHost).toHaveBeenCalledWith('h-1');
  });
});
