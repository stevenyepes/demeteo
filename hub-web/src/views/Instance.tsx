import { type ReactNode, useEffect, useId, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { PresenceDot } from '../components/PresenceDot';
import { HubApiError } from '../lib/api';
import { getInstance, type HubScope, type InstanceDetail, type RunStatus, type RunSummary } from '../lib/fleet';
import { elapsedText, platformText, redactUserinfo } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';

interface InstanceProps {
  instanceId: string;
}

type Load = { state: 'loading' } | { state: 'loaded'; detail: InstanceDetail } | { state: 'failed'; error: unknown };

const SCOPES: { scope: HubScope; meaning: string }[] = [
  { scope: 'read', meaning: 'See its projects, runs and gates' },
  { scope: 'spend', meaning: 'Start and cancel runs that spend provider credit' },
  { scope: 'gates', meaning: 'Decide its gates' },
];

const STATUS_CLASSES: Record<RunStatus, string> = {
  running: 'text-emerald-300',
  completed: 'text-slate-300',
  failed: 'text-ruby-300',
  cancelled: 'text-slate-400',
};

export function Instance({ instanceId }: InstanceProps) {
  const [load, setLoad] = useState<Load>({ state: 'loading' });

  /** Bumped on unmount, so a read that settles after it has no page to land on. */
  const latestRead = useRef(0);

  useEffect(() => {
    latestRead.current += 1;
    const read = latestRead.current;
    setLoad({ state: 'loading' });
    getInstance(instanceId).then(
      (detail) => {
        if (latestRead.current === read) setLoad({ state: 'loaded', detail });
      },
      (error: unknown) => {
        if (latestRead.current === read) setLoad({ state: 'failed', error });
      },
    );
    return () => {
      latestRead.current += 1;
    };
  }, [instanceId]);

  const id = <span className="font-mono text-slate-200 break-all">{instanceId}</span>;

  return (
    <PlaceholderCard title="Instance">
      {load.state === 'loading' && <p>Loading instance {id}…</p>}
      {load.state === 'failed' && isNotFound(load.error) && (
        <p
          role="alert"
          className="max-w-xl rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200 break-words"
        >
          Instance {id} was not found or not yours.
        </p>
      )}
      {load.state === 'failed' && !isNotFound(load.error) && (
        <div className="flex max-w-xl flex-col gap-2">
          <p>Instance {id} could not be loaded.</p>
          <CeremonyError error={load.error} />
        </div>
      )}
      {load.state === 'loaded' && <Loaded detail={load.detail} id={id} />}
    </PlaceholderCard>
  );
}

function isNotFound(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'not_found';
}

function Loaded({ detail, id }: { detail: InstanceDetail; id: ReactNode }) {
  const now = Math.floor(Date.now() / 1000);
  const platform = platformText(detail);
  return (
    <div className="flex flex-col gap-3">
      <div className="nested-card flex flex-wrap items-center gap-3 px-4 py-3">
        {id}
        <span className="flex items-center gap-2 text-xs">
          <PresenceDot present={detail.present} />
          <span aria-hidden="true" className={detail.present ? 'text-emerald-300' : 'text-slate-400'}>
            {detail.present ? 'Online' : 'Offline'}
          </span>
        </span>
        <span className="text-xs text-slate-400">{platform === '' ? 'Platform not reported' : platform}</span>
        <a
          href={routeHref({ name: 'dispatch' })}
          className="btn-primary ml-auto focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
        >
          Run here
        </a>
      </div>
      <Section title="Running now">
        {detail.liveRuns === null ? (
          <Withheld what="Runs" />
        ) : detail.liveRuns.length === 0 ? (
          <p>Nothing running</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {detail.liveRuns.map((run) => (
              <li key={run.featureId}>
                <RunRow run={run} now={now} />
              </li>
            ))}
          </ul>
        )}
      </Section>
      <Section title="Projects">
        {detail.projects === null ? (
          <Withheld what="Projects" />
        ) : (
          <>
            {detail.projects.length === 0 ? (
              <p>No projects reported.</p>
            ) : (
              <ul className="flex flex-col gap-2">
                {detail.projects.map((project) => (
                  <li key={project.id} className="flex flex-col">
                    <span className="text-slate-200 break-all">{project.name}</span>
                    {/* Text, never an `href`: the remote is whatever the machine's git config says. */}
                    {project.remoteUrl === null ? (
                      <span className="text-xs">No remote</span>
                    ) : (
                      <span className="font-mono text-xs break-all">{redactUserinfo(project.remoteUrl)}</span>
                    )}
                  </li>
                ))}
              </ul>
            )}
            <p className="mt-2 text-xs">Local paths stay on the machine.</p>
          </>
        )}
      </Section>
      {/* No Harnesses section: `InstanceDetail.harnesses` is typed `null` until the API reports them. */}
      <Section title="Connection">
        <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1">
          <dt>Present</dt>
          <dd className="text-slate-200">{detail.present ? 'Yes' : 'No'}</dd>
          <dt>Connected since</dt>
          <dd className="text-slate-200">
            {detail.connectedAt === null ? 'Not connected' : `${elapsedText(detail.connectedAt, now)} ago`}
          </dd>
          <dt>Last seen</dt>
          <dd className="text-slate-200">
            {detail.lastSeen === null ? 'Never' : `${elapsedText(detail.lastSeen, now)} ago`}
          </dd>
          <dt>Install id</dt>
          <dd className="text-slate-200">
            {detail.installId === null ? (
              'Not reported'
            ) : (
              <span className="font-mono break-all">{detail.installId}</span>
            )}
          </dd>
        </dl>
      </Section>
      <Section title="What this instance allows">
        <ul className="flex flex-col gap-2">
          {SCOPES.map(({ scope, meaning }) => {
            const allowed = detail.scopes.includes(scope);
            return (
              <li key={scope} className="flex flex-wrap items-baseline gap-x-3">
                <span className="font-mono text-slate-200">{scope}</span>
                <span className="text-xs">{meaning}</span>
                <span className={`ml-auto text-xs ${allowed ? 'text-emerald-300' : 'text-slate-500'}`}>
                  {allowed ? 'Allowed' : 'Not allowed'}
                </span>
              </li>
            );
          })}
        </ul>
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="nested-card px-4 py-3">
      <h2 id={headingId} className="font-heading text-lg font-semibold text-white">
        {title}
      </h2>
      <div className="mt-2">{children}</div>
    </section>
  );
}

/** `null` is withheld without `read` (docs/HUB.md §6.4); an empty list would claim there is nothing. */
function Withheld({ what }: { what: string }) {
  return (
    <p>
      {what} are withheld: read is off on this machine. Turn on read for this Hub on the machine to see them here.
    </p>
  );
}

function RunRow({ run, now }: { run: RunSummary; now: number }) {
  return (
    <a
      href={routeHref({ name: 'run', instanceId: run.instanceId, featureId: run.featureId })}
      className="flex flex-wrap items-baseline gap-x-4 gap-y-1 rounded-lg px-2 py-1 transition-colors hover:bg-cyan-500/10 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400"
    >
      <span className="font-mono text-slate-200 break-all">{run.featureId}</span>
      <span className={`text-xs ${STATUS_CLASSES[run.status]}`}>{run.status}</span>
      {run.startedAt !== null && <span className="text-xs">Started {elapsedText(run.startedAt, now)} ago</span>}
      <span className="text-xs">Updated {elapsedText(run.updatedAt, now)} ago</span>
      {typeof run.costCents === 'number' && <span className="text-xs">${(run.costCents / 100).toFixed(2)}</span>}
    </a>
  );
}
