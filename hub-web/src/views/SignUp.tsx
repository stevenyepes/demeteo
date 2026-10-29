import { type FormEvent, useId, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { type Session, signUp } from '../lib/auth';
import { isUsablePasskeyLabel } from '../lib/passkeyLabel';
import { Button } from '../../../src/components/ui/Button';

interface SignUpProps {
  onSession: (session: Session) => void;
}

export function SignUp({ onSession }: SignUpProps) {
  const labelId = useId();
  const [typed, setTyped] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  const label = typed.trim();
  const canSubmit = !busy && isUsablePasskeyLabel(label);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canSubmit) return;
    setFailure(null);
    setBusy(true);
    let session: Session;
    try {
      session = await signUp(label);
    } catch (error) {
      setFailure({ error });
      return;
    } finally {
      setBusy(false);
    }
    onSession(session);
  }

  return (
    <div className="flex flex-col gap-6">
      <PageHeader title="Sign up" />
      <form onSubmit={submit} className="panel flex flex-col gap-4 p-6">
        <p className="text-sm text-slate-400">
          Your account is its passkeys. Name the first one after the device that holds it.
        </p>
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
        <Button variant="primary" size="lg" type="submit" disabled={!canSubmit}>
          {busy ? 'Waiting for your passkey…' : 'Create passkey'}
        </Button>
        {failure && <CeremonyError error={failure.error} />}
      </form>
    </div>
  );
}
