import { type FormEvent, useId, useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { type Session, signUp } from '../lib/auth';
import { isUsablePasskeyLabel } from '../lib/passkeyLabel';

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
    <PlaceholderCard title="Sign up">
      <p>Your account is its passkeys. Name the first one after the device that holds it.</p>
      <form onSubmit={submit} className="mt-6 flex max-w-sm flex-col gap-4">
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
            className="input-field focus:border-cyan-400! focus:shadow-none!"
          />
        </div>
        <button
          type="submit"
          disabled={!canSubmit}
          className="btn-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {busy ? 'Waiting for your passkey…' : 'Create passkey'}
        </button>
        {failure && <CeremonyError error={failure.error} />}
      </form>
    </PlaceholderCard>
  );
}
