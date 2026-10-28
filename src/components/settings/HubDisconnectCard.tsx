import { useState } from 'react';
import { RotateCw, Unlink } from 'lucide-react';

interface HubDisconnectCardProps {
  disconnecting: boolean;
  onDisconnect: () => void;
}

/** Not a way to stop runs: disconnecting forgets only the pairing, so the copy
 *  must never suggest otherwise. */
export function HubDisconnectCard({ disconnecting, onDisconnect }: HubDisconnectCardProps) {
  const [confirming, setConfirming] = useState(false);

  const confirm = () => {
    setConfirming(false);
    onDisconnect();
  };

  return (
    <div className="glass-panel p-6 rounded-xl space-y-4">
      <div className="flex flex-col md:flex-row md:items-start justify-between gap-4">
        <div>
          <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
            <Unlink className="w-4 h-4 text-ruby-400" /> Disconnect
          </h3>
          <p className="text-xs text-slate-400 leading-relaxed mt-1">
            Forget this pairing on this machine. Pair again to report to a Hub.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setConfirming(true)}
          disabled={disconnecting || confirming}
          className="shrink-0 px-3 py-2 text-xs font-semibold rounded-lg bg-ruby-600 hover:bg-ruby-500 text-white disabled:opacity-40 disabled:hover:bg-ruby-600 flex items-center gap-1.5 transition-all"
        >
          {disconnecting ? (
            <RotateCw className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <Unlink className="w-3.5 h-3.5" />
          )}
          Disconnect from Hub
        </button>
      </div>

      {confirming && (
        <div
          role="alertdialog"
          aria-label="Confirm disconnect"
          className="flex flex-col md:flex-row md:items-center gap-3 p-3 rounded-lg bg-ruby-500/5 border border-ruby-500/20"
        >
          <p className="text-xs text-ruby-200 flex-1 leading-relaxed">
            Disconnecting forgets the device key and tokens on this machine. Runs keep going — this
            does not stop them. The Hub keeps listing this machine until its token expires.
          </p>
          <div className="flex gap-2 shrink-0">
            <button
              type="button"
              onClick={() => setConfirming(false)}
              className="px-3 py-1.5 text-xs rounded-md border border-slate-700/60 text-slate-300 hover:bg-white/5 transition-all"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={confirm}
              className="px-3 py-1.5 text-xs font-semibold rounded-md bg-ruby-600 hover:bg-ruby-500 text-white transition-all"
            >
              Disconnect
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default HubDisconnectCard;
