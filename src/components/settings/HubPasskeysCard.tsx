import { KeySquare } from 'lucide-react';
import type { HubPasskeyOrigin, HubTrustedPasskey } from '../../lib/hub';

const ORIGIN_PILL: Readonly<Record<HubPasskeyOrigin, string>> = {
  pinned: 'bg-white/5 text-slate-300 border-slate-700/60',
  endorsed: 'bg-white/5 text-slate-300 border-slate-700/60',
};

function truncateId(id: string): string {
  return id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-6)}` : id;
}

function signatures(count: number): string {
  return count === 1 ? '1 signature' : `${count} signatures`;
}

interface HubPasskeysCardProps {
  passkeys: HubTrustedPasskey[];
}

export function HubPasskeysCard({ passkeys }: HubPasskeysCardProps) {
  return (
    <div className="panel p-6 space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 flex items-center gap-2">
        <KeySquare className="w-4 h-4 text-slate-400" /> Trusted Hub passkeys
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        A gate decision from the Hub is accepted only when one of these passkeys signed it. The
        pinned passkey came with the pairing; an endorsed one was added after it.
      </p>
      {passkeys.length === 0 ? (
        <p className="text-xs text-slate-500 italic">
          No Hub passkey is trusted yet. Pairing pins the one you approve it with.
        </p>
      ) : (
        <div className="space-y-2">
          {passkeys.map((passkey) => (
            <div
              key={passkey.credential_id}
              data-testid="hub-passkey"
              className="nested-card p-4 flex items-center justify-between gap-4"
            >
              <div className="min-w-0 space-y-1">
                <p
                  title={passkey.credential_id}
                  className="font-mono text-sm text-slate-200 truncate"
                >
                  {truncateId(passkey.credential_id)}
                </p>
                <p className="text-xs text-slate-400">
                  Trusted since {new Date(passkey.pinned_at * 1000).toLocaleDateString()}
                </p>
                <p className="text-xs text-slate-400">{signatures(passkey.sign_count)}</p>
              </div>
              <span
                className={`shrink-0 text-[11px] px-2 py-0.5 rounded-full border font-medium ${ORIGIN_PILL[passkey.origin]}`}
              >
                {passkey.origin}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default HubPasskeysCard;
