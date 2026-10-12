import type { Passkey, PasskeyStatus } from '../lib/passkeys';

const STATUS: Record<PasskeyStatus, { text: string; tone: string }> = {
  trusted: { text: 'Trusted', tone: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-300' },
  pending: { text: 'Pending', tone: 'border-cyan-500/30 bg-cyan-500/10 text-cyan-300' },
  revoked: { text: 'Revoked', tone: 'border-ruby-500/30 bg-ruby-500/10 text-ruby-200' },
};

const CHIP = 'rounded-full border px-2 py-0.5 text-xs font-medium';

const ACTION =
  'rounded-md border px-3 py-1.5 text-xs font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-400 disabled:cursor-not-allowed disabled:opacity-40';

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
        <span className={`${CHIP} border-violet-500/30 bg-violet-500/10 text-violet-200`}>
          This session
        </span>
      )}
      <span className={`${CHIP} ${status.tone}`}>{status.text}</span>
      <div className="ml-auto flex items-center gap-2">
        {passkey.status === 'pending' && (
          <button
            type="button"
            aria-label={`Endorse ${passkey.label}`}
            onClick={() => onEndorse(passkey.credentialId)}
            disabled={disabled}
            className={`${ACTION} border-violet-500/40 bg-violet-500/10 text-violet-200 hover:bg-violet-500/20`}
          >
            Endorse
          </button>
        )}
        {passkey.status !== 'revoked' && (
          <button
            type="button"
            aria-label={`Revoke ${passkey.label}`}
            title={lastTrusted ? 'The only trusted passkey cannot be revoked.' : undefined}
            onClick={() => onRevoke(passkey.credentialId)}
            disabled={disabled || lastTrusted}
            className={`${ACTION} border-ruby-500/30 bg-ruby-500/10 text-ruby-200 hover:bg-ruby-500/20`}
          >
            Revoke
          </button>
        )}
      </div>
    </li>
  );
}
