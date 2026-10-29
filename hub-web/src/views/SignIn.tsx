import { KeyRound } from 'lucide-react';
import { useState } from 'react';

import { CeremonyError } from '../components/CeremonyError';
import { PageHeader } from '../components/PageHeader';
import { type Session, signIn } from '../lib/auth';
import { Button } from '../../../src/components/ui/Button';

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
    <div className="flex flex-col gap-6">
      <PageHeader title="Sign in" />
      <section className="panel flex flex-col gap-4 p-6">
        <p className="text-sm text-slate-400">Use a passkey you have already registered with this Hub.</p>
        <Button variant="primary" size="lg" onClick={start} disabled={busy}>
          <KeyRound aria-hidden="true" className="h-4 w-4" />
          {busy ? 'Waiting for your passkey…' : 'Sign in with a passkey'}
        </Button>
        {failure && <CeremonyError error={failure.error} />}
      </section>
    </div>
  );
}
