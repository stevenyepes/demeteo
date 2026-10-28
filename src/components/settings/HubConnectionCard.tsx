import { useState, type FormEvent, type ReactNode } from 'react';
import { AlertTriangle, Cloud, KeyRound, Link2, RotateCw } from 'lucide-react';
import type { HubConnection, HubConnectionState, HubMachine, HubStatus } from '../../lib/hub';
import { formatEventNumber, stopReasonCopy, syncStateLabel } from '../../lib/hubPresentation';

const STATE_LABELS: Readonly<Record<HubConnectionState, string>> = {
  not_running: 'Not running',
  connecting: 'Connecting',
  connected: 'Connected',
  reconnecting: 'Reconnecting',
  stopped: 'Stopped',
};

function statusDotClass(state: HubConnectionState | undefined): string {
  switch (state) {
    case 'connected':
      return 'w-2 h-2 rounded-full shrink-0 bg-emerald-400 animate-pulse';
    case 'connecting':
    case 'reconnecting':
      return 'w-2 h-2 rounded-full shrink-0 bg-amber-400 animate-pulse';
    case 'stopped':
      return 'w-2 h-2 rounded-full shrink-0 bg-ruby-400';
    default:
      return 'w-2 h-2 rounded-full shrink-0 bg-slate-500';
  }
}

interface HubConnectionCardProps {
  status: HubStatus;
  connection: HubConnection | null;
  machine: HubMachine | null;
  pairing: boolean;
  onPair: (hubUrl: string) => void;
}

export function HubConnectionCard({
  status,
  connection,
  machine,
  pairing,
  onPair,
}: HubConnectionCardProps) {
  return (
    <div className="glass-panel p-6 rounded-xl space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
        <Cloud className="w-4 h-4 text-violet-400" /> Hub connection
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        Report this machine to a Demeteo Hub and let it ask for runs. What it may do is decided
        here, and enforced here.
      </p>
      {status.state === 'unpaired' ? (
        <PairForm pairing={pairing} onPair={onPair} />
      ) : (
        <PairedDetails status={status} connection={connection} machine={machine} />
      )}
      {status.state === 'incomplete' && status.hub_url && (
        <button
          type="button"
          onClick={() => status.hub_url && onPair(status.hub_url)}
          disabled={pairing}
          className="flex items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-lg bg-violet-500/10 border border-violet-500/30 text-violet-300 hover:bg-violet-500/20 transition-all disabled:opacity-50"
        >
          {pairing ? <RotateCw className="w-3.5 h-3.5 animate-spin" /> : <Link2 className="w-3.5 h-3.5" />}
          {pairing ? 'Waiting for the Hub…' : 'Pair again'}
        </button>
      )}
    </div>
  );
}

function PairForm({ pairing, onPair }: { pairing: boolean; onPair: (hubUrl: string) => void }) {
  const [hubUrl, setHubUrl] = useState('');
  const trimmed = hubUrl.trim();

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (trimmed && !pairing) onPair(trimmed);
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      <label htmlFor="hub-url" className="block text-xs text-slate-400">
        Hub URL
      </label>
      <div className="flex items-center gap-2">
        <input
          id="hub-url"
          type="url"
          value={hubUrl}
          onChange={(e) => setHubUrl(e.target.value)}
          placeholder="https://hub.example.com"
          disabled={pairing}
          className="flex-1 bg-black/40 border border-slate-700/60 rounded-lg py-2 px-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600 disabled:opacity-50"
        />
        <button
          type="submit"
          disabled={pairing || trimmed === ''}
          className="shrink-0 flex items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-lg bg-violet-500/10 border border-violet-500/30 text-violet-300 hover:bg-violet-500/20 transition-all disabled:opacity-50"
        >
          {pairing ? <RotateCw className="w-3.5 h-3.5 animate-spin" /> : <Link2 className="w-3.5 h-3.5" />}
          {pairing ? 'Waiting for the Hub…' : 'Pair'}
        </button>
      </div>
      {pairing && (
        <p className="text-xs text-slate-500">
          Approve this machine on the Hub page that opened in your browser.
        </p>
      )}
    </form>
  );
}

function PairedDetails({
  status,
  connection,
  machine,
}: {
  status: HubStatus;
  connection: HubConnection | null;
  machine: HubMachine | null;
}) {
  const latency =
    connection?.latency_ms === null || connection?.latency_ms === undefined
      ? '—'
      : `${connection.latency_ms} ms`;

  return (
    <div className="space-y-3">
      <div className="nested-card p-4 space-y-2">
        <p className="font-mono text-sm text-violet-400 break-all">{status.hub_url}</p>
        <div className="flex items-center gap-2" role="status">
          <span data-testid="hub-status-dot" className={statusDotClass(connection?.state)} />
          <span className="text-xs text-slate-300">
            {connection ? STATE_LABELS[connection.state] : 'State unknown'} · {latency}
          </span>
        </div>
        {connection?.stop_reason && (
          <p className="text-xs text-ruby-400">
            Stopped: {stopReasonCopy(connection.stop_reason)}
          </p>
        )}
      </div>

      <div className="nested-card p-4 space-y-1">
        <p className="text-[11px] text-slate-500 uppercase tracking-wider">
          This machine, as the Hub sees it
        </p>
        {machine ? (
          <>
            <p className="font-heading text-sm font-semibold text-white">{machine.instance_name}</p>
            <p className="text-xs text-slate-400">
              {machine.os} · {machine.arch} · Demeteo {machine.app_version}
            </p>
          </>
        ) : (
          <p className="text-xs text-slate-500 italic">Unknown</p>
        )}
        {status.rp_id && (
          <p className="text-xs text-slate-400">
            Hub passkeys for <span className="font-mono text-slate-300">{status.rp_id}</span>
          </p>
        )}
      </div>

      <div className="nested-card p-4 space-y-2">
        <div className="flex items-start gap-2">
          <KeyRound className="w-4 h-4 text-slate-400 shrink-0 mt-0.5" />
          <div className="min-w-0 space-y-1">
            <p className="text-[11px] text-slate-500 uppercase tracking-wider">Device key</p>
            <p className="font-mono text-xs text-slate-300 break-all">
              {status.fingerprint ?? '—'}
            </p>
            <p className="text-xs text-slate-400">
              Kept in {status.keyring_backend} · never in SQLite
            </p>
          </div>
        </div>
        {!status.keyring_available && (
          <KeyringNote>
            The {status.keyring_backend} is unavailable, so this desktop cannot read its device key
            or tokens.
          </KeyringNote>
        )}
        {status.keyring_available && status.state === 'incomplete' && (
          <KeyringNote>
            The pairing is incomplete: part of it is missing from the {status.keyring_backend}. Pair
            again to repair it.
          </KeyringNote>
        )}
      </div>

      <div className="nested-card p-4">
        <p className="text-xs text-slate-300">
          {typeof connection?.reported_through === 'number'
            ? `Reported up to event ${formatEventNumber(connection.reported_through)}`
            : 'Nothing acknowledged since this app started'}{' '}
          · {syncStateLabel(connection?.sync ?? 'offline')}
        </p>
      </div>
    </div>
  );
}

function KeyringNote({ children }: { children: ReactNode }) {
  return (
    <div
      role="alert"
      className="bg-amber-500/10 border border-amber-500/20 p-3 rounded-lg flex items-start gap-3"
    >
      <AlertTriangle className="w-4 h-4 text-amber-400 shrink-0 mt-0.5" />
      <span className="text-xs text-amber-300">{children}</span>
    </div>
  );
}

export default HubConnectionCard;
