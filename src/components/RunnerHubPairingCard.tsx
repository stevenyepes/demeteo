import { useState, type FormEvent } from 'react';
import { AlertTriangle, CircleDot, Cloud, Copy, ExternalLink, KeyRound, Link2, RotateCw, X } from 'lucide-react';
import type { HubConnectionState } from '../lib/hub';
import type { DeviceTicket, RunnerHubStatus, RunnerPairState } from '../lib/runnerHub';
import { stopReasonCopy, syncStateLabel } from '../lib/hubPresentation';
import { TONE_TEXT, type RunStatusTone } from '../lib/runStatus';
import { Button, buttonClasses } from './ui/Button';

const STATE_LABELS: Readonly<Record<HubConnectionState, string>> = {
  not_running: 'Not running',
  connecting: 'Connecting',
  connected: 'Connected',
  reconnecting: 'Reconnecting',
  stopped: 'Stopped',
};

/** Reconnecting is slate: the runner retries on its own and needs nobody. */
const STATE_TONES: Readonly<Record<HubConnectionState, RunStatusTone>> = {
  not_running: 'slate',
  connecting: 'cyan',
  connected: 'emerald',
  reconnecting: 'slate',
  stopped: 'ruby',
};

interface RunnerHubPairingCardProps {
  hub: RunnerHubStatus;
  pairState: RunnerPairState;
  /** The runner's own refusal of the last `hub_pair_begin`, unchanged. */
  pairError: string | null;
  busy: boolean;
  onPair: (hubUrl: string) => void;
  onCancel: () => void;
}

export function RunnerHubPairingCard({
  hub,
  pairState,
  pairError,
  busy,
  onPair,
  onCancel,
}: RunnerHubPairingCardProps) {
  const { status } = hub;
  const pending = pairState.state === 'pending';
  return (
    <div className="panel p-6 space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 flex items-center gap-2">
        <Cloud className="w-4 h-4 text-slate-400" /> Hub connection
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        Report this runner to a Demeteo Hub and let it ask for runs. What it may do is decided
        here, and enforced on the runner.
      </p>
      {hub.store_unavailable && (
        <div role="alert" className="nested-card p-4 flex items-start gap-3">
          <AlertTriangle className="w-4 h-4 text-amber-400 shrink-0 mt-0.5" />
          <div className="min-w-0 space-y-1">
            <p className="text-xs text-slate-200">The runner cannot keep Hub credentials yet</p>
            <p className="font-mono text-xs text-slate-400 break-words">{hub.store_unavailable}</p>
          </div>
        </div>
      )}
      {pending ? (
        <TicketDetails ticket={pairState.ticket} busy={busy} onCancel={onCancel} />
      ) : status.state === 'paired' ? (
        <PairedDetails hub={hub} />
      ) : (
        <>
          {status.state === 'incomplete' && <PairedDetails hub={hub} />}
          <PairForm
            initialUrl={status.hub_url ?? ''}
            again={status.state === 'incomplete'}
            busy={busy}
            onPair={onPair}
          />
        </>
      )}
      {pairState.state === 'failed' && (
        <p role="alert" className="text-xs text-ruby-400 break-words">
          Pairing failed: {pairState.error}
        </p>
      )}
      {pairError && (
        <p role="alert" className="text-xs text-ruby-400 break-words">
          {pairError}
        </p>
      )}
    </div>
  );
}

function PairForm({
  initialUrl,
  again,
  busy,
  onPair,
}: {
  initialUrl: string;
  again: boolean;
  busy: boolean;
  onPair: (hubUrl: string) => void;
}) {
  const [hubUrl, setHubUrl] = useState(initialUrl);
  const trimmed = hubUrl.trim();

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (trimmed && !busy) onPair(trimmed);
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-2">
      <label htmlFor="runner-hub-url" className="block text-xs text-slate-400">
        Hub URL
      </label>
      <div className="flex items-center gap-2">
        <input
          id="runner-hub-url"
          type="url"
          value={hubUrl}
          onChange={(e) => setHubUrl(e.target.value)}
          placeholder="https://hub.example.com"
          disabled={busy}
          className="flex-1 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white font-mono focus:outline-none focus:border-accent-ring placeholder-slate-600 disabled:opacity-50"
        />
        <Button type="submit" variant="primary" disabled={busy || trimmed === ''} className="shrink-0">
          {busy ? <RotateCw className="w-3.5 h-3.5 animate-spin" /> : <Link2 className="w-3.5 h-3.5" />}
          {again ? 'Pair again' : 'Pair'}
        </Button>
      </div>
    </form>
  );
}

function TicketDetails({
  ticket,
  busy,
  onCancel,
}: {
  ticket: DeviceTicket;
  busy: boolean;
  onCancel: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(ticket.verification_uri_complete);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="space-y-3" data-testid="runner-hub-ticket">
      <div className="nested-card p-4 space-y-2">
        <p className="text-xs text-slate-400">Enter this code on the Hub to approve the runner</p>
        <p className="font-mono text-2xl font-semibold text-white">{ticket.user_code}</p>
        <div className="flex flex-wrap items-center gap-2">
          <a
            href={ticket.verification_uri_complete}
            target="_blank"
            rel="noopener noreferrer"
            className={buttonClasses('secondary', 'sm')}
          >
            <ExternalLink className="w-3.5 h-3.5" /> Open the Hub
          </a>
          <Button size="sm" variant="ghost" onClick={copy}>
            <Copy className="w-3.5 h-3.5" /> {copied ? 'Copied' : 'Copy link'}
          </Button>
        </div>
        <p className="font-mono text-xs text-slate-400 break-all">{ticket.verification_uri}</p>
        <p className="text-xs text-slate-400">
          Expires at {new Date(ticket.expires_at * 1000).toLocaleTimeString()}
        </p>
      </div>
      <div className="nested-card p-4 flex items-start gap-2">
        <KeyRound className="w-4 h-4 text-slate-400 shrink-0 mt-0.5" />
        <div className="min-w-0 space-y-1">
          <p className="text-xs text-slate-400">
            The Hub shows the runner's device key beside the code. Approve only if they match.
          </p>
          <p className="font-mono text-xs text-slate-300 break-all">{ticket.fingerprint}</p>
        </div>
      </div>
      <div className="flex items-center gap-2">
        <RotateCw className="w-3.5 h-3.5 text-cyan-400 animate-spin" aria-hidden="true" />
        <span className="text-xs text-slate-400 flex-1">Waiting for the Hub to approve</span>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={busy}>
          <X className="w-3.5 h-3.5" /> Cancel pairing
        </Button>
      </div>
    </div>
  );
}

function PairedDetails({ hub }: { hub: RunnerHubStatus }) {
  const { status, connection, machine } = hub;
  const tone = STATE_TONES[connection.state];
  return (
    <div className="space-y-3">
      <div className="nested-card p-4 space-y-2">
        <p className="font-mono text-sm text-slate-200 break-all">{status.hub_url}</p>
        <p className="flex items-center gap-2 text-xs text-slate-300" role="status">
          <CircleDot className={`w-3.5 h-3.5 shrink-0 ${TONE_TEXT[tone]}`} aria-hidden="true" />
          {STATE_LABELS[connection.state]}
          {connection.latency_ms !== null && (
            <span className="font-mono text-slate-400">{connection.latency_ms} ms</span>
          )}
          <span className="text-slate-500">{syncStateLabel(connection.sync)}</span>
        </p>
        {connection.stop_reason && (
          <p className="text-xs text-ruby-400">Stopped: {stopReasonCopy(connection.stop_reason)}</p>
        )}
      </div>
      <div className="nested-card p-4 space-y-1">
        <p className="text-xs text-slate-500">This runner, as the Hub sees it</p>
        <p className="font-heading text-sm font-semibold text-white">{machine.instance_name}</p>
        <p className="text-xs text-slate-400">
          {machine.os} · {machine.arch} · Demeteo {machine.app_version}
        </p>
        <p className="font-mono text-xs text-slate-400 break-all">{status.fingerprint ?? '—'}</p>
      </div>
    </div>
  );
}

export default RunnerHubPairingCard;
