import { type Guard, getJson } from './api';
import { postInSession } from './auth';

/** A GitHub App connected from this Hub, as `GET /v1/git-hosts` lists it. */
export type GitHost = {
  id: string;
  host: string;
  appId: number;
  appSlug: string;
  /** The GitHub account or organization that owns the App. */
  account: string | null;
  /** GitHub's page for choosing the repositories the App is installed on. */
  installUrl: string;
  /** Unix seconds. */
  connectedAt: number;
};

export type GitHosts = {
  /** Hosts an App can be connected on, github.com first. */
  connectable: string[];
  hosts: GitHost[];
};

/** A repository one of the connected Apps is installed on, as `GET /v1/git-hosts/repositories` lists it. */
export type ReachableRepository = {
  /** Lowercase, as a Hub project spells it. */
  host: string;
  /** `owner/repo`. */
  fullName: string;
  private: boolean;
  defaultBranch: string | null;
  description: string | null;
};

export type RepositoryListing = {
  repositories: ReachableRepository[];
  /** Apps that did not answer, so their repositories are missing from the list. */
  unlisted: { host: string; appId: number }[];
  /** More repositories exist than the Hub read. */
  truncated: boolean;
};

/** What the browser posts to GitHub to create the App. */
export type ManifestForm = {
  actionUrl: string;
  manifest: string;
  state: string;
};

type WireGitHost = {
  id: string;
  host: string;
  app_id: number;
  app_slug: string;
  account: string | null;
  install_url: string;
  connected_at: number;
};

export async function listGitHosts(): Promise<GitHosts> {
  const wire = await getJson('/v1/git-hosts', isWireGitHosts);
  return {
    connectable: wire.connectable,
    hosts: wire.hosts.map((host) => ({
      id: host.id,
      host: host.host,
      appId: host.app_id,
      appSlug: host.app_slug,
      account: host.account,
      installUrl: host.install_url,
      connectedAt: host.connected_at,
    })),
  };
}

export async function listRepositories(): Promise<RepositoryListing> {
  const wire = await getJson('/v1/git-hosts/repositories', isWireListing);
  return {
    repositories: wire.repositories.map((repo) => ({
      host: repo.host,
      fullName: repo.full_name,
      private: repo.private,
      defaultBranch: repo.default_branch,
      description: repo.description,
    })),
    unlisted: wire.unlisted.map((app) => ({ host: app.host, appId: app.app_id })),
    truncated: wire.truncated,
  };
}

/** `organization` empty or absent creates the App under the signed-in GitHub user. */
export async function startManifest(host: string, organization: string): Promise<ManifestForm> {
  const body = organization.trim() === '' ? { host } : { host, organization: organization.trim() };
  const wire = await postInSession('/v1/git-hosts/github/manifest', body, isWireManifestForm);
  return { actionUrl: wire.action_url, manifest: wire.manifest, state: wire.state };
}

export async function removeGitHost(id: string): Promise<void> {
  await postInSession(`/v1/git-hosts/${encodeURIComponent(id)}/remove`, {}, isRemoved);
}

/**
 * GitHub's manifest flow starts with a form POST from the browser: a fetch
 * cannot do it, since GitHub answers with its own page for the user to
 * confirm. `state` rides in the query, where GitHub expects it.
 */
export function submitManifest(form: ManifestForm, doc: Document = document): void {
  const element = doc.createElement('form');
  element.method = 'post';
  element.action = `${form.actionUrl}?state=${encodeURIComponent(form.state)}`;
  element.hidden = true;
  const field = doc.createElement('input');
  field.type = 'hidden';
  field.name = 'manifest';
  field.value = form.manifest;
  element.appendChild(field);
  doc.body.appendChild(element);
  element.submit();
}

const isWireGitHost: Guard<WireGitHost> = (value): value is WireGitHost => {
  const account = memberOf(value, 'account');
  return (
    typeof memberOf(value, 'id') === 'string' &&
    typeof memberOf(value, 'host') === 'string' &&
    typeof memberOf(value, 'app_id') === 'number' &&
    typeof memberOf(value, 'app_slug') === 'string' &&
    (account === null || typeof account === 'string') &&
    isHttpsUrl(memberOf(value, 'install_url')) &&
    typeof memberOf(value, 'connected_at') === 'number'
  );
};

const isWireGitHosts: Guard<{ connectable: string[]; hosts: WireGitHost[] }> = (
  value,
): value is { connectable: string[]; hosts: WireGitHost[] } => {
  const connectable = memberOf(value, 'connectable');
  const hosts = memberOf(value, 'hosts');
  return (
    Array.isArray(connectable) &&
    connectable.every((host) => typeof host === 'string') &&
    Array.isArray(hosts) &&
    hosts.every(isWireGitHost)
  );
};

/**
 * The action is where the browser is sent, so only an `https` URL is taken:
 * anything else from a Hub answering oddly would be a navigation this page
 * did not mean to make.
 */
const isWireManifestForm: Guard<{ action_url: string; manifest: string; state: string }> = (
  value,
): value is { action_url: string; manifest: string; state: string } =>
  isHttpsUrl(memberOf(value, 'action_url')) &&
  typeof memberOf(value, 'manifest') === 'string' &&
  typeof memberOf(value, 'state') === 'string';

type WireRepository = {
  host: string;
  full_name: string;
  private: boolean;
  default_branch: string | null;
  description: string | null;
};

type WireListing = {
  repositories: WireRepository[];
  unlisted: { host: string; app_id: number }[];
  truncated: boolean;
};

const isWireRepository = (value: unknown): value is WireRepository =>
  typeof memberOf(value, 'host') === 'string' &&
  typeof memberOf(value, 'full_name') === 'string' &&
  typeof memberOf(value, 'private') === 'boolean' &&
  isStringOrNull(memberOf(value, 'default_branch')) &&
  isStringOrNull(memberOf(value, 'description'));

const isWireListing: Guard<WireListing> = (value): value is WireListing => {
  const repositories = memberOf(value, 'repositories');
  const unlisted = memberOf(value, 'unlisted');
  return (
    Array.isArray(repositories) &&
    repositories.every(isWireRepository) &&
    Array.isArray(unlisted) &&
    unlisted.every((app) => typeof memberOf(app, 'host') === 'string' && typeof memberOf(app, 'app_id') === 'number') &&
    typeof memberOf(value, 'truncated') === 'boolean'
  );
};

function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

const isRemoved: Guard<{ removed: true }> = (value): value is { removed: true } =>
  memberOf(value, 'removed') === true;

function isHttpsUrl(value: unknown): value is string {
  if (typeof value !== 'string') return false;
  try {
    return new URL(value).protocol === 'https:';
  } catch {
    return false;
  }
}

function memberOf(value: unknown, member: string): unknown {
  return typeof value === 'object' && value !== null ? Reflect.get(value, member) : undefined;
}
