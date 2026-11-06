import { type FormEvent, useEffect, useId, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { PairingClaims } from '../components/PairingClaims';
import { PairingScopes } from '../components/PairingScopes';
import { HubApiError } from '../lib/api';
import { approveDevice, type DevicePrompt, denyDevice, lookUpDevice } from '../lib/devicePairing';
import type { PairingScope } from '../lib/pairing';
import { routeHref } from '../lib/router';
import { Button } from '../../../src/components/ui/Button';

const HINTS: Record<string, string> = {
  pairing_expired:
    'No pending pairing has this code. It may have expired or been answered already. Check the code, or run demeteo-runner hub pair again.',
  too_many_lookups: 'Too many codes were entered that match no pairing. Wait ten minutes, then try again.',
  invalid_request: 'That is not a code a runner prints. It is eight letters in two groups, like BCDF-GHJK.',
};

/** Every refusal the Hub gives this page that a different code, or a new one, is the answer to. */
function hintOf(error: unknown): string | null {
  return error instanceof HubApiError && error.code !== null ? (HINTS[error.code] ?? null) : null;
}

const ANSWERED = {
  approved: 'Approved. The runner finishes pairing the next time it checks, within a few seconds.',
  denied: 'Denied. The runner is told so the next time it checks.',
} as const;

interface ApproveDeviceProps {
  /** Absent at `#/device`, where the human types the code the runner printed. */
  userCode?: string;
}

export function ApproveDevice({ userCode }: ApproveDeviceProps) {
  const [prompt, setPrompt] = useState<DevicePrompt | null>(null);
  /** `PairingScopes` says why this holds what was taken away. */
  const [withheld, setWithheld] = useState<PairingScope[]>([]);
  const [busy, setBusy] = useState(false);
  const [answered, setAnswered] = useState<keyof typeof ANSWERED | null>(null);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  useEffect(() => {
    if (userCode === undefined) return;
    let shown = true;
    lookUpDevice(userCode).then(
      (found) => {
        if (shown) setPrompt(found);
      },
      (error: unknown) => {
        if (shown) setFailure({ error });
      },
    );
    return () => {
      shown = false;
    };
  }, [userCode]);

  const granted = prompt?.scopes.filter((scope) => !withheld.includes(scope)) ?? [];
  /**
   * A code the Hub no longer holds, or will not look up, cannot be answered
   * from here. A refused assertion, a cancelled prompt or an expired ceremony
   * leave the pairing pending, and Approve may be tried again.
   */
  const hint = hintOf(failure?.error);
  const answerable = !busy && answered === null && hint === null;

  function toggle(scope: PairingScope, checked: boolean) {
    setWithheld((held) => (checked ? held.filter((other) => other !== scope) : [...held, scope]));
  }

  async function answer(decision: () => Promise<void>, as: keyof typeof ANSWERED) {
    setFailure(null);
    setBusy(true);
    try {
      await decision();
      setAnswered(as);
    } catch (error) {
      setFailure({ error });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Pair runner"
        eyebrow={
          <a href={routeHref({ name: 'add-instance' })} className="transition-colors hover:text-white">
            Add instance
          </a>
        }
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        A demeteo-runner host printed a code for you to approve here. Approving it pairs that host
        with your account on this Hub.
      </p>
      {userCode === undefined && <CodeEntry />}
      {userCode !== undefined && prompt === null && failure === null && (
        <p className="text-sm text-slate-400">Looking up the code…</p>
      )}
      {prompt !== null && (
        <div className="panel flex max-w-2xl flex-col gap-6 p-6">
          <p className="text-sm text-slate-300">
            Code <span className="font-mono text-white">{prompt.userCode}</span>
          </p>
          <PairingClaims
            kind="runner"
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
              onClick={() => answer(() => approveDevice(prompt.userCode, granted), 'approved')}
              disabled={!answerable || granted.length === 0}
            >
              Approve with passkey
            </Button>
            <Button
              variant="danger-subtle"
              onClick={() => answer(() => denyDevice(prompt.userCode), 'denied')}
              disabled={!answerable}
            >
              Deny
            </Button>
            {busy && (
              <p role="status" className="text-sm text-slate-400">
                Sending your answer…
              </p>
            )}
            {answered !== null && (
              <p role="status" className="text-sm text-slate-300">
                {ANSWERED[answered]}
              </p>
            )}
          </div>
        </div>
      )}
      {failure && (
        <div className="flex max-w-2xl flex-col gap-3">
          <CeremonyError error={failure.error} />
          {hint !== null && <p className="text-xs text-slate-300">{hint}</p>}
          {hint !== null && (
            <a href={routeHref({ name: 'device' })} className="text-sm text-accent-ink hover:underline">
              Enter a different code
            </a>
          )}
        </div>
      )}
    </div>
  );
}

/** Puts the code in the hash, so what is shown next is the page `verification_uri_complete` opens. */
function CodeEntry() {
  const id = useId();
  const [entered, setEntered] = useState('');
  const code = entered.trim();

  function submit(event: FormEvent) {
    event.preventDefault();
    if (code !== '') window.location.hash = routeHref({ name: 'device', userCode: code });
  }

  return (
    <form onSubmit={submit} className="panel flex max-w-md flex-col gap-3 p-6">
      <label htmlFor={`${id}-code`} className="text-sm text-slate-300">
        Code from the runner
      </label>
      <input
        id={`${id}-code`}
        value={entered}
        onChange={(event) => setEntered(event.target.value)}
        autoComplete="off"
        autoCapitalize="characters"
        spellCheck={false}
        placeholder="BCDF-GHJK"
        className="input-field font-mono uppercase"
      />
      <div>
        <Button variant="primary" type="submit" disabled={code === ''}>
          Continue
        </Button>
      </div>
    </form>
  );
}
