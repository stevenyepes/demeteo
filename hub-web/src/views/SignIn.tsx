import { useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PlaceholderCard } from '../components/PlaceholderCard';
import { type Session, signIn } from '../lib/auth';

interface SignInProps {
  onSession: (session: Session) => void;
}

export function SignIn({ onSession }: SignInProps) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ error: unknown } | null>(null);

  async function start() {
    setFailure(null);
    setBusy(true);
    let session: Session;
    try {
      session = await signIn();
    } catch (error) {
      setFailure({ error });
      return;
    } finally {
      setBusy(false);
    }
    onSession(session);
  }

  return (
    <PlaceholderCard title="Sign in">
      <p>Use a passkey you have already registered with this Hub.</p>
      <div className="mt-6 flex max-w-sm flex-col gap-4">
        <button
          type="button"
          onClick={start}
          disabled={busy}
          className="btn-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {busy ? 'Waiting for your passkey…' : 'Sign in with a passkey'}
        </button>
        {failure && <CeremonyError error={failure.error} />}
      </div>
    </PlaceholderCard>
  );
}
