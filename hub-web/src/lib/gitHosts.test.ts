import { afterEach, describe, expect, it, vi } from 'vitest';

import { dropSession, HubApiError } from './api';
import { listGitHosts, listRepositories, removeGitHost, startManifest, submitManifest } from './gitHosts';

type Exchange = { method: 'GET' | 'POST'; path: string; answer: Response };
type Sent = { path: string; body: unknown };

/** A request that is not the next one scripted throws, so a call this file did not expect fails the test. */
function stubFetch(script: Exchange[]) {
  const pending = [...script];
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const next = pending.shift();
      const method = init?.method ?? 'GET';
      if (next === undefined || next.path !== input || next.method !== method) {
        throw new Error(`fetch double was not told about ${method} ${String(input)} here`);
      }
      sent.push({
        path: next.path,
        body: typeof init?.body === 'string' ? (JSON.parse(init.body) as unknown) : undefined,
      });
      return next.answer;
    }),
  );
  return { sent, pending };
}

function answer(method: 'GET' | 'POST', path: string, body: unknown, status = 200): Exchange {
  return {
    method,
    path,
    answer: new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } }),
  };
}

const WIRE_HOST = {
  id: '3f2b1c3e-6a1f-4b8e-9c3d-2a1b0c9d8e7f',
  host: 'github.com',
  app_id: 5647010,
  app_slug: 'demeteo-hub-example',
  account: 'octo-org',
  install_url: 'https://github.com/apps/demeteo-hub-example/installations/new',
  connected_at: 1_762_400_000,
};

afterEach(() => {
  vi.unstubAllGlobals();
  dropSession();
});

describe('listGitHosts', () => {
  it('reads the hosts into camelCase', async () => {
    stubFetch([answer('GET', '/v1/git-hosts', { connectable: ['github.com'], hosts: [WIRE_HOST] })]);

    await expect(listGitHosts()).resolves.toEqual({
      connectable: ['github.com'],
      hosts: [
        {
          id: WIRE_HOST.id,
          host: 'github.com',
          appId: 5647010,
          appSlug: 'demeteo-hub-example',
          account: 'octo-org',
          installUrl: WIRE_HOST.install_url,
          connectedAt: 1_762_400_000,
        },
      ],
    });
  });

  it('refuses an install link that is not https', async () => {
    stubFetch([
      answer('GET', '/v1/git-hosts', {
        connectable: ['github.com'],
        hosts: [{ ...WIRE_HOST, install_url: 'javascript:alert(1)' }],
      }),
    ]);

    await expect(listGitHosts()).rejects.toBeInstanceOf(HubApiError);
  });
});

describe('listRepositories', () => {
  it('reads the repositories and the Apps that did not answer', async () => {
    stubFetch([
      answer('GET', '/v1/git-hosts/repositories', {
        repositories: [
          { host: 'github.com', full_name: 'acme/widgets', private: true, default_branch: 'main', description: null },
        ],
        unlisted: [{ host: 'ghe.example', app_id: 11 }],
        truncated: false,
      }),
    ]);

    await expect(listRepositories()).resolves.toEqual({
      repositories: [
        { host: 'github.com', fullName: 'acme/widgets', private: true, defaultBranch: 'main', description: null },
      ],
      unlisted: [{ host: 'ghe.example', appId: 11 }],
      truncated: false,
    });
  });

  it('refuses a listing missing its unlisted Apps', async () => {
    stubFetch([answer('GET', '/v1/git-hosts/repositories', { repositories: [], truncated: false })]);

    await expect(listRepositories()).rejects.toBeInstanceOf(HubApiError);
  });
});

describe('startManifest', () => {
  const FORM = { action_url: 'https://github.com/settings/apps/new', manifest: '{}', state: 's-1' };

  it('sends no organization when none was typed', async () => {
    const { sent } = stubFetch([answer('POST', '/v1/git-hosts/github/manifest', FORM)]);

    await expect(startManifest('github.com', '  ')).resolves.toEqual({
      actionUrl: FORM.action_url,
      manifest: '{}',
      state: 's-1',
    });
    expect(sent[0].body).toEqual({ host: 'github.com' });
  });

  it('sends the organization trimmed', async () => {
    const { sent } = stubFetch([answer('POST', '/v1/git-hosts/github/manifest', FORM)]);

    await startManifest('github.com', ' octo-org ');
    expect(sent[0].body).toEqual({ host: 'github.com', organization: 'octo-org' });
  });

  it('will not send the browser to an action that is not https', async () => {
    stubFetch([
      answer('POST', '/v1/git-hosts/github/manifest', { ...FORM, action_url: 'http://github.com/settings/apps/new' }),
    ]);

    await expect(startManifest('github.com', '')).rejects.toBeInstanceOf(HubApiError);
  });
});

describe('removeGitHost', () => {
  it('names the host in the path and takes the removal as answered', async () => {
    const { pending } = stubFetch([answer('POST', `/v1/git-hosts/${WIRE_HOST.id}/remove`, { removed: true })]);

    await removeGitHost(WIRE_HOST.id);
    expect(pending).toHaveLength(0);
  });
});

describe('submitManifest', () => {
  it('posts the manifest to GitHub with the state in the query', () => {
    const submit = vi.spyOn(HTMLFormElement.prototype, 'submit').mockImplementation(() => {});

    submitManifest({ actionUrl: 'https://github.com/settings/apps/new', manifest: '{"a":1}', state: 'a b' });

    const form = document.body.querySelector('form');
    expect(form?.method).toBe('post');
    expect(form?.action).toBe('https://github.com/settings/apps/new?state=a%20b');
    expect(form?.querySelector('input[name="manifest"]')).toHaveValue('{"a":1}');
    expect(submit).toHaveBeenCalledOnce();
    form?.remove();
    submit.mockRestore();
  });
});
