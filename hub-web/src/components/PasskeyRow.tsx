import { KeyRound } from 'lucide-react';

import { Button } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import type { RunStatusTone } from '../../../src/lib/runStatus';
import type { Passkey, PasskeyStatus } from '../lib/passkeys';

/** A pending passkey waits for a trusted one to endorse it, so it is the one amber state here. */
const STATUS: Record<PasskeyStatus, { text: string; tone: RunStatusTone }> = {
  trusted: { text: 'Trusted', tone: 'emerald' },
  pending: { text: 'Pending', tone: 'amber' },
  revoked: { text: 'Revoked', tone: 'ruby' },
};

interface PasskeyRowProps {
  passkey: Passkey;
  /** Whether it is the only trusted passkey, which the Hub refuses to revoke (`last_passkey`). */
  lastTrusted: boolean;
  disabled: boolean;
  onEndorse: (credentialId: string) => void;
  onRevoke: (credentialId: string) => void;
}

export function PasskeyRow({ passkey, lastTrusted, disabled, onEndorse, onRevoke }: PasskeyRowProps) {
  const status = STATUS[passkey.status];
  return (
    <li className="flex flex-wrap items-center gap-3 px-4 py-3">
      <KeyRound aria-hidden="true" className="h-4 w-4 shrink-0 text-slate-500" />
      <span className="font-medium text-slate-100 break-all">{passkey.label}</span>
      {passkey.current && <span className="text-xs text-slate-400">This session</span>}
      <Chip tone={status.tone} dot={false}>
        {status.text}
      </Chip>
      <div className="ml-auto flex items-center gap-2">
        {passkey.status === 'pending' && (
          <Button
            variant="primary"
            size="sm"
            aria-label={`Endorse ${passkey.label}`}
            onClick={() => onEndorse(passkey.credentialId)}
            disabled={disabled}
          >
            Endorse
          </Button>
        )}
        {passkey.status !== 'revoked' && (
          <Button
            variant="danger-subtle"
            size="sm"
            aria-label={`Revoke ${passkey.label}`}
            title={lastTrusted ? 'The only trusted passkey cannot be revoked.' : undefined}
            onClick={() => onRevoke(passkey.credentialId)}
            disabled={disabled || lastTrusted}
          >
            Revoke
          </Button>
        )}
      </div>
    </li>
  );
}
