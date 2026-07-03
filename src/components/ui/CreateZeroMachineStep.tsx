import { AlertCircle, CheckCircle2, HardDrive, Key, Loader2, Server } from 'lucide-react';
import type { Machine } from '../../types';
import type { MachineProbeStatus } from './useCreateZeroWizardForm';

export interface CreateZeroMachineStepProps {
  machineKind: 'local' | 'remote';
  machineId: string;
  machines: ReadonlyArray<Machine>;
  keyPassphrase: string;
  /** Live status of the `test_machine_connection` probe for the
   *  currently selected remote machine. Surfaces inline so the
   *  user knows whether the wizard's **Next** control is blocked
   *  on a failing SSH probe, never on a silent local fallback. */
  probeStatus: MachineProbeStatus;
  probeError: string | null;
  onRetest: () => void;
  onMachineKindChange: (kind: 'local' | 'remote') => void;
  onMachineIdChange: (id: string) => void;
  onKeyPassphraseChange: (value: string) => void;
}

/**
 * Step 3 — local vs remote. Renders the kind toggle, the SSH machine
 * dropdown (only when remote is chosen), and the key-passphrase
 * sub-field (only when the chosen machine uses key auth).
 *
 * When the user picks a remote machine, the parent's form runs a
 *  `test_machine_connection` probe; this component surfaces the
 *  probe status inline so the user sees why **Next** is disabled and
 *  can re-trigger the probe after fixing credentials/network.
 */
export function CreateZeroMachineStep(props: CreateZeroMachineStepProps) {
  const {
    machineKind, machineId, machines, keyPassphrase,
    probeStatus, probeError, onRetest,
    onMachineKindChange, onMachineIdChange, onKeyPassphraseChange,
  } = props;
  const selectedMachine = machines.find((m) => m.id === machineId) ?? null;
  const showPassphrase = machineKind === 'remote' && selectedMachine?.auth_type === 'key';
  const showProbe = machineKind === 'remote' && Boolean(machineId);

  return (
    <div className="space-y-4">
      <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block">
        Where should agents run?
      </label>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <button
          type="button"
          onClick={() => { onMachineKindChange('local'); onMachineIdChange(''); }}
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            machineKind === 'local'
              ? 'bg-violet-500/10 border-violet-500/50 text-violet-100'
              : 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
          }`}
        >
          <HardDrive className="w-4 h-4" />
          <div>
            <div className="font-medium">Local compute</div>
            <div className="text-[10px] font-mono text-slate-500">Run on this workstation</div>
          </div>
        </button>
        <button
          type="button"
          onClick={() => onMachineKindChange('remote')}
          className={`flex items-center gap-3 p-3 rounded-lg border text-left text-sm transition-all ${
            machineKind === 'remote'
              ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-100'
              : 'bg-black/40 border-white/10 text-slate-300 hover:border-white/20'
          }`}
        >
          <Server className="w-4 h-4" />
          <div>
            <div className="font-medium">Remote SSH</div>
            <div className="text-[10px] font-mono text-slate-500">Run on a configured machine</div>
          </div>
        </button>
      </div>

      {machineKind === 'remote' && (
        <div className="space-y-2">
          <select
            value={machineId}
            onChange={(e) => onMachineIdChange(e.target.value)}
            data-testid="wizard-machine-select"
            className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white font-mono focus:outline-none focus:border-cyan-500/50"
          >
            <option value="">
              {machines.length === 0 ? 'No machines configured — add one in Settings → Machines' : 'Select a machine…'}
            </option>
            {machines.map((m) => (
              <option key={m.id} value={m.id}>
                {m.name} ({m.username}@{m.host}:{m.port} — {m.auth_type})
              </option>
            ))}
          </select>
          {showPassphrase && (
            <div>
              <label className="block text-[10px] font-mono text-slate-400 uppercase tracking-widest mb-1.5 flex items-center gap-1.5">
                <Key className="w-3 h-3" /> Private key passphrase
              </label>
              <input
                type="password"
                value={keyPassphrase}
                onChange={(e) => onKeyPassphraseChange(e.target.value)}
                placeholder="Leave blank if the key has no passphrase"
                autoComplete="off"
                className="w-full bg-black/40 border border-white/10 rounded-lg p-2.5 text-xs text-white font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
              />
            </div>
          )}

          {showProbe && (
            <div
              data-testid="wizard-machine-probe"
              className={`flex items-start gap-2 rounded-lg px-3 py-2 text-[11px] font-mono border ${
                probeStatus === 'success'
                  ? 'border-emerald-500/40 bg-emerald-500/10 text-emerald-200'
                  : probeStatus === 'error'
                    ? 'border-ruby-500/40 bg-ruby-500/10 text-ruby-200'
                    : probeStatus === 'running'
                      ? 'border-cyan-500/40 bg-cyan-500/10 text-cyan-200'
                      : 'border-white/10 bg-black/30 text-slate-400'
              }`}
            >
              {probeStatus === 'running' && <Loader2 className="w-3.5 h-3.5 animate-spin mt-0.5 shrink-0" />}
              {probeStatus === 'success' && <CheckCircle2 className="w-3.5 h-3.5 mt-0.5 shrink-0" />}
              {probeStatus === 'error' && <AlertCircle className="w-3.5 h-3.5 mt-0.5 shrink-0" />}
              <div className="min-w-0 flex-1">
                {probeStatus === 'running' && (
                  <span>Probing SSH connection to <code>{selectedMachine?.name}</code>…</span>
                )}
                {probeStatus === 'success' && (
                  <span>Connected. <code>{selectedMachine?.name}</code> accepted credentials.</span>
                )}
                {probeStatus === 'error' && (
                  <>
                    <div className="font-semibold">Connection probe failed — Next is disabled.</div>
                    <div className="mt-0.5 break-words opacity-80">{probeError ?? 'Unknown error'}</div>
                  </>
                )}
                {probeStatus === 'idle' && (
                  <span>Select a machine to probe its SSH connection.</span>
                )}
              </div>
              {(probeStatus === 'error' || probeStatus === 'success') && (
                <button
                  type="button"
                  onClick={onRetest}
                  className="ml-2 text-[10px] underline opacity-80 hover:opacity-100"
                >
                  {probeStatus === 'error' ? 'Retry probe' : 'Re-test'}
                </button>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
