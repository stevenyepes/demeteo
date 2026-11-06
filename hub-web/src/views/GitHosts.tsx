import { type FormEvent, useCallback, useEffect, useId, useRef, useState } from 'react';

import { Button, buttonClasses } from '../../../src/components/ui/Button';
import { CeremonyError } from '../components/CeremonyError';
import { GitHostRow } from '../components/GitHostRow';
import { PageHeader } from '../components/PageHeader';
import { type GitHosts as Listed, listGitHosts, removeGitHost, startManifest, submitManifest } from '../lib/gitHosts';

/**
 * What the Hub's redirect back from GitHub can say went wrong, keyed by the
 * code in `crates/demeteo-hub/src/git_hosts/manifest.rs`.
 */
const FAILURES: Record<string, string> = {
  expired:
    'That return from GitHub was already used, or came back more than 30 minutes after you started. Connect again.',
  conversion_failed:
    'GitHub did not hand over the new App. If GitHub shows it was created, delete it there, then connect again.',
  key_refused: "GitHub's key for the new App is not one this Hub can use.",
  import_forbidden:
    "This Hub's key service does not allow it to store App keys yet. Whoever runs the Hub has to grant the import paths in its OpenBao policy, then you can connect again.",
  key_service_unavailable: "This Hub's key service is sealed or unreachable. Connect again once it is back.",
  store_failed: 'The App was created but this Hub could not record it. Connect again.',
};

function failureOf(code: string): string {
  return FAILURES[code] ?? 'Connecting the App did not finish. Connect again.';
}

interface GitHostsProps {
  /** The id the Hub's redirect named after connecting an App. */
  connected?: string;
  /** The code the Hub's redirect named when connecting failed. */
  failed?: string;
}

/**
 * Runs this Hub starts on a runner push their branch and open their pull
 * request as a GitHub App the user owns. Connecting one sends the browser to
 * GitHub with a prefilled App, and GitHub sends it back to the Hub, which
 * stores the App's key in its key service and lands here.
 */
export function GitHosts({ connected, failed }: GitHostsProps) {
  const listId = useId();
  const formId = useId();
  const hostId = useId();
  const orgId = useId();
  const [listed, setListed] = useState<Listed | null>(null);
  const [host, setHost] = useState('');
  const [organization, setOrganization] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);
  const latestRead = useRef(0);

  const load = useCallback(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    listGitHosts().then(
      (answer) => {
        if (latestRead.current !== read) return;
        setListed(answer);
        setHost((chosen) => chosen || (answer.connectable[0] ?? ''));
      },
      (error: unknown) => {
        if (latestRead.current === read) setFailure({ error });
      },
    );
  }, []);

  useEffect(() => {
    load();
    return () => {
      latestRead.current += 1;
    };
  }, [load]);

  async function connect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || host === '') return;
    setFailure(null);
    setBusy(true);
    try {
      submitManifest(await startManifest(host, organization));
    } catch (error) {
      setFailure({ error });
      setBusy(false);
    }
  }

  async function remove(id: string) {
    setFailure(null);
    setBusy(true);
    try {
      await removeGitHost(id);
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
      load();
    }
  }

  const justConnected = connected === undefined ? undefined : listed?.hosts.find((h) => h.id === connected);

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Git hosts"
        summary={listed === null ? undefined : `${listed.hosts.length} connected`}
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        Runs this Hub starts on a runner push their branch and open their pull request as a GitHub
        App you own. The Hub keeps the App's key in its key service and never reads it back.
      </p>
      {justConnected !== undefined && (
        <section aria-label="App connected" className="panel flex max-w-2xl flex-col gap-3 p-5">
          <p className="text-sm text-slate-200">
            <span className="font-mono">{justConnected.appSlug}</span> is connected. Choose the
            repositories it may reach. Runs on any other repository wait until you add it.
          </p>
          <a href={justConnected.installUrl} className={`${buttonClasses('primary', 'md')} self-start`}>
            Choose repositories on GitHub
          </a>
        </section>
      )}
      {failed !== undefined && (
        <div className="max-w-2xl">
          <CeremonyError error={new Error(failureOf(failed))} />
        </div>
      )}
      {listed === null && failure === null && <p className="text-sm text-slate-400">Loading git hosts…</p>}
      {listed !== null && (
        <div className="flex flex-wrap items-start gap-6">
          <section aria-labelledby={listId} className="panel min-w-0 flex-[2_1_480px] p-2">
            <h2 id={listId} className="px-4 pt-3 pb-2 font-heading text-base font-semibold text-white">
              Connected Apps
            </h2>
            {listed.hosts.length === 0 ? (
              <p className="px-4 pb-4 text-sm text-slate-400">
                None yet. Until you connect one, runs this Hub starts on a runner wait before they push.
              </p>
            ) : (
              <ul className="flex flex-col divide-y divide-hairline">
                {listed.hosts.map((gitHost) => (
                  <GitHostRow key={gitHost.id} host={gitHost} disabled={busy} onRemove={remove} />
                ))}
              </ul>
            )}
          </section>
          <form
            onSubmit={connect}
            aria-labelledby={formId}
            className="panel flex min-w-0 flex-[1_1_300px] flex-col gap-4 p-5"
          >
            <h2 id={formId} className="font-heading text-base font-semibold text-white">
              Connect GitHub
            </h2>
            {listed.connectable.length > 1 && (
              <div className="flex flex-col gap-1.5">
                <label htmlFor={hostId} className="text-xs font-medium text-slate-300">
                  Host
                </label>
                <select
                  id={hostId}
                  value={host}
                  onChange={(event) => setHost(event.target.value)}
                  className="input-field"
                >
                  {listed.connectable.map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
              </div>
            )}
            <div className="flex flex-col gap-1.5">
              <label htmlFor={orgId} className="text-xs font-medium text-slate-300">
                Organization (optional)
              </label>
              <input
                id={orgId}
                type="text"
                value={organization}
                onChange={(event) => setOrganization(event.target.value)}
                autoComplete="off"
                spellCheck={false}
                className="input-field font-mono"
              />
              <p className="text-xs text-slate-400">Leave empty to create the App under your own account.</p>
            </div>
            <Button variant="primary" type="submit" disabled={busy || host === ''}>
              Connect GitHub
            </Button>
            <p className="text-xs text-slate-400">
              GitHub opens with the App filled in: private, and allowed only to push code and open
              pull requests. You confirm it there.
            </p>
          </form>
        </div>
      )}
      {failure && (
        <div className="max-w-xl">
          <CeremonyError error={failure.error} />
          {listed === null && (
            <Button
              variant="primary"
              onClick={() => {
                setFailure(null);
                load();
              }}
              className="mt-4"
            >
              Try again
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
