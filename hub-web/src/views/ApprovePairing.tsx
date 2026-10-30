import { useEffect, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { PairingClaims } from '../components/PairingClaims';
import { PairingScopes } from '../components/PairingScopes';
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
  const [prompt, setPrompt] = useState<PairingPrompt | null>(null);
  /** `PairingScopes` says why this holds what was taken away. */
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
          <PairingClaims
            kind="desktop"
            instanceName={prompt.instanceName}
            os={prompt.os}
            arch={prompt.arch}
            appVersion={prompt.appVersion}
            fingerprint={prompt.fingerprint}
          />
          <PairingScopes
            requested={prompt.scopes}
            withheld={withheld}
            onToggle={toggle}
            disabled={!answerable}
          />
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
