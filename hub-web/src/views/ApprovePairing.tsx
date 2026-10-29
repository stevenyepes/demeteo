import { Monitor } from 'lucide-react';
import { useEffect, useId, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { HubApiError } from '../lib/api';
import {
  approvePairing,
  denyPairing,
  leaveFor,
  type PairingPrompt,
  type PairingScope,
  startPairing,
} from '../lib/pairing';
import { routeHref } from '../lib/router';
import { Button } from '../../../src/components/ui/Button';

/** What each scope allows, as `docs/HUB.md` §7 states it. */
const ALLOWS: Record<PairingScope, string> = {
  read: 'This instance sends its events and snapshots to the Hub.',
  spend: 'The Hub may start, cancel and re-point runs that spend money.',
  gates: 'The Hub may relay a gate decision to this instance.',
};

const EXPIRED_HINT =
  'This pairing request has expired or was already answered. Start pairing again from Demeteo on the machine.';
const SPENT_HINT =
  'This pairing request can no longer be answered. Start pairing again from Demeteo on the machine.';

/**
 * What to tell the user when the Hub no longer holds the prompt, or null while
 * it still does. The Hub takes a prompt for the first answer its handler looks
 * at, not only for one it accepts: an approval refused as `invalid_scope`, one
 * that found the code store full (`too_many_pairings`) and one that failed
 * with a `500` have each used the prompt up, exactly as `pairing_expired` says
 * of one that timed out or was answered before. No second answer can land
 * after any of them, and a Deny could no longer reach the machine.
 *
 * Every other failure was turned away before the handler ran and leaves the
 * prompt where it was: no response at all, a `401`, the CSRF `403`, and
 * `400 invalid_request` for a body the Hub could not read.
 */
function spentHintOf(error: unknown): string | null {
  if (!(error instanceof HubApiError)) return null;
  if (error.code === 'pairing_expired') return EXPIRED_HINT;
  const spent = error.code === 'invalid_scope' || error.code === 'too_many_pairings' || error.status === 500;
  return spent ? SPENT_HINT : null;
}

interface ApprovePairingProps {
  request: string;
}

export function ApprovePairing({ request }: ApprovePairingProps) {
  const id = useId();
  const [prompt, setPrompt] = useState<PairingPrompt | null>(null);
  /**
   * What the user took away, not what they kept: the scopes approved are the
   * requested ones less these, so no state this view can reach holds a scope
   * the machine did not ask for.
   */
  const [withheld, setWithheld] = useState<PairingScope[]>([]);
  const [busy, setBusy] = useState(false);
  const [answered, setAnswered] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  useEffect(() => {
    let shown = true;
    startPairing(request).then(
      (started) => {
        if (shown) setPrompt(started);
      },
      (error: unknown) => {
        if (shown) setFailure({ error });
      },
    );
    return () => {
      shown = false;
    };
  }, [request]);

  const granted = prompt?.scopes.filter((scope) => !withheld.includes(scope)) ?? [];
  /** A refused start parked nothing, so only a failure over a prompt can have spent one. */
  const spentHint = prompt === null ? null : spentHintOf(failure?.error);
  const answerable = !busy && !answered && spentHint === null;

  function toggle(scope: PairingScope, checked: boolean) {
    setWithheld((held) => (checked ? held.filter((other) => other !== scope) : [...held, scope]));
  }

  async function answer(decision: () => Promise<string>) {
    setFailure(null);
    setBusy(true);
    try {
      leaveFor(await decision());
      setAnswered(true);
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Pair instance"
        eyebrow={
          <a href={routeHref({ name: 'add-instance' })} className="transition-colors hover:text-white">
            Add instance
          </a>
        }
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        A Demeteo instance is asking to be paired with your account on this Hub.
      </p>
      {prompt === null && failure === null && <p className="text-sm text-slate-400">Reading the pairing request…</p>}
      {prompt !== null && (
        <div className="panel flex max-w-2xl flex-col gap-6 p-6">
          <section aria-labelledby={`${id}-claimed`} className="flex items-start gap-4">
            <span
              aria-hidden="true"
              className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border border-hairline bg-white/5 text-slate-300"
            >
              <Monitor className="h-5 w-5" />
            </span>
            <div className="flex min-w-0 flex-1 flex-col gap-2">
              <h2 id={`${id}-claimed`} className="text-xs font-medium text-slate-400">
                Claimed by the machine
              </h2>
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
                <dt className="text-slate-400">Name</dt>
                <dd className="font-heading font-semibold text-white break-all">{prompt.instanceName}</dd>
                <dt className="text-slate-400">OS</dt>
                <dd className="font-mono text-slate-200 break-all">{prompt.os}</dd>
                <dt className="text-slate-400">Architecture</dt>
                <dd className="font-mono text-slate-200 break-all">{prompt.arch}</dd>
                <dt className="text-slate-400">Demeteo version</dt>
                <dd className="font-mono text-slate-200 break-all">{prompt.appVersion}</dd>
              </dl>
              <p className="text-xs text-slate-400">
                The machine reported these about itself. The Hub has not verified them.
              </p>
            </div>
          </section>
          <section
            aria-labelledby={`${id}-key`}
            className="flex flex-col gap-2 rounded-md border border-hairline bg-surface-well px-4 py-3"
          >
            <h2 id={`${id}-key`} className="text-xs font-medium text-slate-400">
              Device key fingerprint
            </h2>
            <p className="font-mono text-lg text-white break-all">{prompt.fingerprint}</p>
            <p className="text-xs text-slate-400">
              Check that this matches the fingerprint Demeteo shows on the machine. If it does not,
              choose Deny.
            </p>
          </section>
          <fieldset className="flex flex-col">
            <legend className="text-xs font-medium text-slate-400">Scopes requested</legend>
            <p className="mt-1 mb-2 text-xs text-slate-400">
              Uncheck any you do not want to grant. Nothing the machine did not ask for can be added
              here.
            </p>
            <div className="flex flex-col divide-y divide-hairline rounded-md border border-hairline">
              {prompt.scopes.map((scope) => (
                <div key={scope} className="flex items-start gap-3 px-4 py-3">
                  <input
                    id={`${id}-${scope}`}
                    type="checkbox"
                    checked={!withheld.includes(scope)}
                    onChange={(event) => toggle(scope, event.target.checked)}
                    disabled={!answerable}
                    aria-describedby={`${id}-${scope}-allows`}
                    className="mt-0.5 accent-accent disabled:cursor-not-allowed disabled:opacity-40"
                  />
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <label htmlFor={`${id}-${scope}`} className="font-mono text-sm text-slate-200">
                      {scope}
                    </label>
                    <span id={`${id}-${scope}-allows`} className="text-xs text-slate-400">
                      {ALLOWS[scope]}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </fieldset>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="primary"
              onClick={() => answer(() => approvePairing(prompt.pairingId, granted))}
              disabled={!answerable || granted.length === 0}
            >
              Approve
            </Button>
            <Button
              variant="danger-subtle"
              onClick={() => answer(() => denyPairing(prompt.pairingId))}
              disabled={!answerable}
            >
              Deny
            </Button>
            {busy && (
              <p role="status" className="text-sm text-slate-400">
                Sending your answer…
              </p>
            )}
            {answered && (
              <p role="status" className="text-sm text-slate-400">
                Returning to Demeteo on the machine…
              </p>
            )}
          </div>
        </div>
      )}
      {failure && (
        <div className="max-w-2xl">
          <CeremonyError error={failure.error} />
          {spentHint !== null && <p className="mt-3 text-xs text-slate-300">{spentHint}</p>}
        </div>
      )}
    </div>
  );
}
