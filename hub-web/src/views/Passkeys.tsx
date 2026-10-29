import { type FormEvent, useCallback, useEffect, useId, useRef, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PasskeyRow } from '../components/PasskeyRow';
import { PageHeader } from '../components/PageHeader';
import { HubApiError } from '../lib/api';
import { isUsablePasskeyLabel } from '../lib/passkeyLabel';
import {
  addPasskey,
  endorsePasskey,
  listPasskeys,
  type Passkey,
  revokePasskey,
} from '../lib/passkeys';
import { Button } from '../../../src/components/ui/Button';

/**
 * `changed` is the passkey as the Hub answered it, and it is the only way a
 * row's status moves here. The view never writes the status it asked for: a
 * row reading `Trusted` is the page's whole statement that an assertion was
 * verified, and only the Hub can make it.
 */
function withPasskey(held: Passkey[], changed: Passkey): Passkey[] {
  return held.some((passkey) => passkey.credentialId === changed.credentialId)
    ? held.map((passkey) => (passkey.credentialId === changed.credentialId ? changed : passkey))
    : [...held, changed];
}

/**
 * The Hub holds a bounded number of ceremonies per signed-in passkey and evicts
 * that passkey's own oldest to make room (`MAX_LIVE_PER_HOLDER` in
 * `crates/demeteo-hub/src/auth/ceremonies.rs`). So a second sign-in with the
 * same passkey can displace a request in flight here, which no sign-in with a
 * different passkey can do. The bare code tells the user none of that.
 */
function isExpiredCeremony(error: unknown): boolean {
  return error instanceof HubApiError && error.code === 'ceremony_expired';
}

function showNothing(): void {}

export function Passkeys() {
  const labelId = useId();
  const listId = useId();
  const addId = useId();
  const [passkeys, setPasskeys] = useState<Passkey[] | null>(null);
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  /**
   * Numbers the reads of the list so that only the latest one is applied. One
   * that settles after a newer one started carries the older answer, and one
   * that settles after the view unmounted has no page to put it on.
   */
  const latestRead = useRef(0);

  const readList = useCallback((onRefused: (error: unknown) => void) => {
    latestRead.current += 1;
    const read = latestRead.current;
    listPasskeys().then(
      (listed) => {
        if (latestRead.current === read) setPasskeys(listed);
      },
      (error: unknown) => {
        if (latestRead.current === read) onRefused(error);
      },
    );
  }, []);

  const load = useCallback(() => readList((error) => setFailure({ error })), [readList]);

  useEffect(() => {
    load();
    return () => {
      latestRead.current += 1;
    };
  }, [load]);

  function retryLoad() {
    setFailure(null);
    load();
  }

  const label = typed.trim();
  const canAdd = !busy && isUsablePasskeyLabel(label);

  /**
   * One at a time: a browser shows a single WebAuthn prompt, so two ceremonies cannot both be answered.
   *
   * However the attempt settles, the list is read again. The answered passkey
   * is one row; the others move without this page too, and a finish whose
   * answer was lost may have committed. That read says nothing of its own: the
   * attempt's outcome is the only thing shown about the attempt, and a read
   * answered `401` has already taken the shell to the sign-in view in
   * `lib/api.ts`.
   */
  async function run(ceremony: () => Promise<Passkey>): Promise<boolean> {
    setFailure(null);
    setBusy(true);
    try {
      const changed = await ceremony();
      setPasskeys((held) => held && withPasskey(held, changed));
      return true;
    } catch (error) {
      setFailure({ error });
      return false;
    } finally {
      setBusy(false);
      readList(showNothing);
    }
  }

  async function add(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canAdd) return;
    if (await run(() => addPasskey(label))) setTyped('');
  }

  const trusted = passkeys?.filter((passkey) => passkey.status === 'trusted').length ?? 0;

  const pending = passkeys?.filter((passkey) => passkey.status === 'pending').length ?? 0;

  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Passkeys"
        summary={passkeys === null ? undefined : summaryOf(trusted, pending)}
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        Each trusted passkey signs you in to this Hub. Revoking one signs it out everywhere.
      </p>
      {passkeys === null && failure === null && <p className="text-sm text-slate-400">Loading your passkeys…</p>}
      {passkeys !== null && (
        <div className="flex flex-wrap items-start gap-6">
          <section aria-labelledby={listId} className="panel min-w-0 flex-[2_1_480px] p-2">
            <h2 id={listId} className="sr-only">
              Your passkeys
            </h2>
            <ul className="flex flex-col divide-y divide-hairline">
              {passkeys.map((passkey) => (
                <PasskeyRow
                  key={passkey.credentialId}
                  passkey={passkey}
                  lastTrusted={passkey.status === 'trusted' && trusted === 1}
                  disabled={busy}
                  onEndorse={(credentialId) => run(() => endorsePasskey(credentialId))}
                  onRevoke={(credentialId) => run(() => revokePasskey(credentialId))}
                />
              ))}
            </ul>
            {pending > 0 && (
              <p className="border-t border-hairline px-4 py-3 text-xs text-slate-400">
                A pending passkey cannot sign in yet. A passkey that is already trusted must approve
                it: choose Endorse on its row and answer the prompt with a trusted passkey.
              </p>
            )}
          </section>
          <form
            onSubmit={add}
            aria-labelledby={addId}
            className="panel flex min-w-0 flex-[1_1_280px] flex-col gap-4 p-5"
          >
            <h2 id={addId} className="font-heading text-base font-semibold text-white">
              Add a passkey
            </h2>
            <div className="flex flex-col gap-1.5">
              <label htmlFor={labelId} className="text-xs font-medium text-slate-300">
                Passkey name
              </label>
              <input
                id={labelId}
                type="text"
                value={typed}
                onChange={(event) => setTyped(event.target.value)}
                autoComplete="off"
                className="input-field"
              />
            </div>
            <Button variant="primary" type="submit" disabled={!canAdd}>
              Add passkey
            </Button>
            <p className="text-xs text-slate-400">It starts pending until a trusted passkey endorses it.</p>
          </form>
        </div>
      )}
      {busy && (
        <p role="status" className="text-sm text-slate-400">
          Waiting for your passkey…
        </p>
      )}
      {failure && (
        <div className="max-w-xl">
          <CeremonyError error={failure.error} />
          {isExpiredCeremony(failure.error) && (
            <p className="mt-3 text-xs text-slate-400">
              The Hub was no longer holding this request. Try again. If it keeps happening, another
              sign-in with this same passkey may be starting requests of its own: sign in with a
              different trusted passkey and revoke this one.
            </p>
          )}
          {passkeys === null && (
            <Button variant="primary" onClick={retryLoad} className="mt-4">
              Try again
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

function summaryOf(trusted: number, pending: number): string {
  const parts = [`${trusted} trusted`];
  if (pending > 0) parts.push(`${pending} pending`);
  return parts.join(' · ');
}
