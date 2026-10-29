import { type ReactNode, useEffect, useId, useRef, useState } from 'react';

import { Check, Minus } from 'lucide-react';

import { buttonClasses } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import { CeremonyError } from '../components/CeremonyError';
import { InstanceProjectsTable } from '../components/InstanceProjectsTable';
import { InstanceDetailRunRow } from '../components/InstanceDetailRunRow';
import { PageHeader } from '../components/PageHeader';
import { HubApiError } from '../lib/api';
import { getInstance, type HubScope, type InstanceDetail } from '../lib/fleet';
import { useFleet } from '../lib/fleetContext';
import { elapsedText, platformText } from '../lib/fleetSummary';
import { routeHref } from '../lib/router';

interface InstanceProps {
  instanceId: string;
}

type Load = { state: 'loading' } | { state: 'loaded'; detail: InstanceDetail } | { state: 'failed'; error: unknown };

const SCOPES: { scope: HubScope; meaning: string }[] = [
  { scope: 'read', meaning: 'See its projects, runs, step progress, feature titles and gates' },
  { scope: 'spend', meaning: 'Start and cancel runs that spend provider credit' },
  { scope: 'gates', meaning: 'Decide its gates, each decision signed with your passkey' },
];

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

  if (load.state === 'loaded') return <Loaded detail={load.detail} />;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Instance" />
      <section className="panel p-6 text-sm text-slate-400">
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
      </section>
    </div>
  );
}

function isNotFound(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'not_found';
}

function Loaded({ detail }: { detail: InstanceDetail }) {
  const { fleet } = useFleet();
  const now = Math.floor(Date.now() / 1000);
  const platform = platformText(detail);
  const fleetEntry = fleet?.find((instance) => instance.id === detail.id);
  const label = fleetEntry?.label ?? null;
  const gateSince = new Map<string, number>();
  for (const gate of fleetEntry?.needsYou ?? []) {
    const known = gateSince.get(gate.featureId);
    if (known === undefined || gate.since < known) gateSince.set(gate.featureId, gate.since);
  }
  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        eyebrow={
          <nav aria-label="Breadcrumb" className="flex flex-wrap gap-2">
            <a href={routeHref({ name: 'fleet' })} className="transition-colors hover:text-white">
              Fleet
            </a>
            <span aria-hidden="true">/</span>
            <span aria-current="page" className="text-slate-200">
              {label ?? detail.id}
            </span>
          </nav>
        }
        title={label ?? 'Instance'}
        summary={platform === '' ? 'Platform not reported' : platform}
        actions={
          <>
            <Chip tone={detail.present ? 'emerald' : 'slate'} dot className="self-center">
              {detail.present ? 'Online' : 'Offline'}
            </Chip>
            <a href={routeHref({ name: 'dispatch', instanceId: detail.id })} className={buttonClasses('primary')}>
              Run here
            </a>
          </>
        }
      />
      <div className="flex flex-wrap items-start gap-6">
        <div className="flex min-w-0 flex-[999_1_560px] flex-col gap-6">
          <Section title="Running now">
            {detail.liveRuns === null ? (
              <Withheld what="Runs" />
            ) : detail.liveRuns.length === 0 ? (
              <p>Nothing running</p>
            ) : (
              <ul className="flex flex-col gap-3">
                {detail.liveRuns.map((run) => (
                  <li key={run.featureId}>
                    <InstanceDetailRunRow run={run} gateSince={gateSince.get(run.featureId) ?? null} now={now} />
                  </li>
                ))}
              </ul>
            )}
          </Section>
          <Section title="Projects" note={detail.projects === null ? undefined : 'Names and remotes only. Local paths stay on the machine.'}>
            {detail.projects === null ? (
              <Withheld what="Projects" />
            ) : detail.projects.length === 0 ? (
              <p>No projects reported.</p>
            ) : (
              <InstanceProjectsTable projects={detail.projects} workflows={detail.workflows} runShape={detail.runShape} />
            )}
          </Section>
          <Section title="Harnesses">
            {detail.harnesses === null ? (
              detail.readGranted ? (
                <p>This instance has not reported its harnesses.</p>
              ) : (
                <Withheld what="Harnesses" />
              )
            ) : detail.harnesses.length === 0 ? (
              <p>No harnesses registered.</p>
            ) : (
              <ul className="flex flex-wrap gap-2">
                {detail.harnesses.map((harness) => (
                  <li key={harness} className="nested-card px-3 py-1.5 font-mono text-xs text-slate-200">
                    {harness}
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
        <div className="flex min-w-0 flex-[1_1_320px] flex-col gap-6">
          <Section title="Connection">
            <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2">
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
          <Section
            title="What this instance allows"
            note="Granted on the machine itself. To widen it, change it in Demeteo there."
          >
            <ul className="flex flex-col gap-3">
              {SCOPES.map(({ scope, meaning }) => {
                const allowed = detail.scopes.includes(scope);
                const Mark = allowed ? Check : Minus;
                return (
                  <li key={scope} className="flex items-start gap-3">
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <span className={`font-mono ${allowed ? 'text-slate-200' : 'text-slate-400'}`}>{scope}</span>
                      <span className="text-xs">{meaning}</span>
                    </span>
                    <span className={`flex shrink-0 items-center gap-1 text-xs ${allowed ? 'text-slate-200' : 'text-slate-400'}`}>
                      <Mark aria-hidden="true" className="h-3.5 w-3.5" />
                      {allowed ? 'Allowed' : 'Not allowed'}
                    </span>
                  </li>
                );
              })}
            </ul>
          </Section>
        </div>
      </div>
    </div>
  );
}

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  const headingId = useId();
  return (
    <section aria-labelledby={headingId} className="panel flex flex-col gap-3 p-5 text-sm text-slate-400">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <h2 id={headingId} className="font-heading text-base font-semibold text-white">
          {title}
        </h2>
        {note !== undefined && <p className="text-xs">{note}</p>}
      </div>
      {children}
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
