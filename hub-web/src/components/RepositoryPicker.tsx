import { Box, Lock, Search } from 'lucide-react';
import { useId, useState } from 'react';

import { buttonClasses } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import type { ReachableRepository, RepositoryListing } from '../lib/gitHosts';
import { routeHref } from '../lib/router';

const GITHUB = 'github.com';

/** The key a picked repository is held by: hosts and owners compare without case, as GitHub's do. */
export function repositoryKey(host: string, fullName: string): string {
  return `${host.toLowerCase()}/${fullName.toLowerCase()}`;
}

interface RepositoryPickerProps {
  /** `null` while the Hub is still asking GitHub. */
  listing: RepositoryListing | null;
  /** True once at least one App is connected; with none, the list cannot hold anything. */
  hostsConnected: boolean;
  /** Keys of repositories that already have a Hub project, offered but not choosable. */
  taken: ReadonlySet<string>;
  selectedKey: string | null;
  onSelect: (repository: ReachableRepository) => void;
  /** Shown under the list when a save was refused for want of a repository. */
  error?: string;
}

/**
 * Only what a connected App is installed on is offered: a repository typed
 * by hand is one a runner may have no token for, and the run would find
 * that out only after it was dispatched.
 */
export function RepositoryPicker({ listing, hostsConnected, taken, selectedKey, onSelect, error }: RepositoryPickerProps) {
  const ids = { heading: useId(), search: useId(), error: useId() };
  const [query, setQuery] = useState('');
  const needle = query.trim().toLowerCase();
  const shown =
    listing?.repositories.filter((repo) => needle === '' || `${repo.host}/${repo.fullName}`.toLowerCase().includes(needle)) ??
    [];

  return (
    <section aria-labelledby={ids.heading} className="panel flex min-w-0 flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 id={ids.heading} className="font-heading text-base font-semibold text-white">
          Repository
        </h2>
        <p className="text-sm text-slate-400">Repositories your connected GitHub Apps are installed on.</p>
      </div>
      {!hostsConnected ? (
        <EmptyState text="No Git host is connected yet. Connect a GitHub App, install it on your repositories, then come back." />
      ) : listing === null ? (
        <p className="text-sm text-slate-400">Asking GitHub for your repositories…</p>
      ) : listing.repositories.length === 0 && listing.unlisted.length === 0 ? (
        <EmptyState text="Your GitHub Apps are not installed on any repository yet. Choose repositories for an App, then come back." />
      ) : (
        <>
          <div className="relative">
            <Search aria-hidden="true" className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-slate-500" />
            <label htmlFor={ids.search} className="sr-only">
              Search repositories
            </label>
            <input
              id={ids.search}
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search repositories"
              autoComplete="off"
              spellCheck={false}
              className="input-field pl-9"
            />
          </div>
          {shown.length === 0 ? (
            <p className="text-sm text-slate-400">
              {listing.repositories.length === 0 ? 'No repositories could be listed.' : 'No repository matches that search.'}
            </p>
          ) : (
            <ul
              aria-labelledby={ids.heading}
              aria-describedby={error ? ids.error : undefined}
              className="flex max-h-[26rem] flex-col gap-1.5 overflow-y-auto pr-1"
            >
              {shown.map((repo) => {
                const key = repositoryKey(repo.host, repo.fullName);
                return (
                  <li key={key}>
                    <RepositoryRow
                      repository={repo}
                      selected={key === selectedKey}
                      taken={taken.has(key)}
                      onSelect={() => onSelect(repo)}
                    />
                  </li>
                );
              })}
            </ul>
          )}
          {listing.unlisted.length > 0 && (
            <p className="text-xs text-slate-400">{unlistedSentence(listing.unlisted.length)}</p>
          )}
          {listing.truncated && (
            <p className="text-xs text-slate-400">The list stops at 1000 repositories an App can reach. Search narrows it.</p>
          )}
        </>
      )}
      {error && (
        <p id={ids.error} className="text-xs text-ruby-300">
          {error}
        </p>
      )}
    </section>
  );
}

function unlistedSentence(count: number): string {
  return count === 1
    ? 'One GitHub App could not be asked for its repositories, so the list may be missing some. Try again in a moment.'
    : `${count} GitHub Apps could not be asked for their repositories, so the list may be missing some. Try again in a moment.`;
}

function EmptyState({ text }: { text: string }) {
  return (
    <div className="nested-card flex flex-col items-start gap-3 p-4">
      <p className="text-sm text-slate-300">{text}</p>
      <a href={routeHref({ name: 'git-hosts' })} className={buttonClasses('secondary', 'sm')}>
        Open Git hosts
      </a>
    </div>
  );
}

interface RepositoryRowProps {
  repository: ReachableRepository;
  selected: boolean;
  taken: boolean;
  onSelect: () => void;
}

function RepositoryRow({ repository, selected, taken, onSelect }: RepositoryRowProps) {
  const id = useId();
  const slash = repository.fullName.lastIndexOf('/');
  const owner = repository.fullName.slice(0, slash + 1);
  const name = repository.fullName.slice(slash + 1);
  const details = [repository.host === GITHUB ? null : repository.host, repository.defaultBranch, repository.description]
    .filter((part): part is string => typeof part === 'string' && part !== '')
    .join(' · ');

  return (
    <button
      type="button"
      disabled={taken}
      aria-pressed={selected}
      aria-labelledby={`${id}-name`}
      aria-describedby={taken ? `${id}-taken` : undefined}
      onClick={onSelect}
      className={`flex w-full items-start gap-3 rounded-lg border px-3 py-2.5 text-left transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${
        selected ? 'border-accent-edge bg-accent-tint' : 'border-transparent enabled:hover:bg-white/5'
      }`}
    >
      <Box aria-hidden="true" className={`mt-0.5 h-4 w-4 shrink-0 ${selected ? 'text-accent-ink' : 'text-slate-500'}`} />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex flex-wrap items-center gap-2">
          <span id={`${id}-name`} className="font-mono text-sm break-all">
            <span className="text-slate-400">{owner}</span>
            <span className={selected ? 'text-white' : 'text-slate-100'}>{name}</span>
          </span>
          {repository.private && (
            <Chip tone="slate" icon={<Lock aria-hidden="true" className="h-3 w-3" />}>
              private
            </Chip>
          )}
        </span>
        {details !== '' && <span className="text-xs text-slate-400 break-words">{details}</span>}
        {taken && (
          <span id={`${id}-taken`} className="text-xs text-slate-400">
            Already a Hub project
          </span>
        )}
      </span>
    </button>
  );
}
