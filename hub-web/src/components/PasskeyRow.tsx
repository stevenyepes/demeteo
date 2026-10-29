import { Button } from '../../../src/components/ui/Button';
import { TONE_CHIP, type RunStatusTone } from '../../../src/lib/runStatus';
import type { Passkey, PasskeyStatus } from '../lib/passkeys';

/** A pending passkey waits for a trusted one to endorse it, so it is the one amber state here. */
const STATUS: Record<PasskeyStatus, { text: string; tone: RunStatusTone }> = {
  trusted: { text: 'Trusted', tone: 'emerald' },
  pending: { text: 'Pending', tone: 'amber' },
  revoked: { text: 'Revoked', tone: 'ruby' },
};

const CHIP = 'rounded-full border px-2 py-0.5 text-xs font-medium';

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
    <li className="nested-card flex flex-wrap items-center gap-3 px-4 py-3">
      <span className="font-medium text-slate-200 break-all">{passkey.label}</span>
      {passkey.current && (
        <span className={`${CHIP} border-accent-edge bg-accent-tint text-accent-ink`}>
          This session
        </span>
      )}
      <span className={`${CHIP} ${TONE_CHIP[status.tone]}`}>{status.text}</span>
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
