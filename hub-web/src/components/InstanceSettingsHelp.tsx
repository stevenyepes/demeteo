import type { InstanceKind } from '../lib/fleet';
import { PairingCopyButton } from './PairingCopyButton';

/** What the instance is missing: a scope switched off, or a ceiling unset or too low. */
export type InstanceSettingNeed = 'spend' | 'gates' | 'ceiling';

/**
 * A runner command that turns `need` on. The ceilings are an example the
 * person is told to change, not a default: they are the instance's own
 * limit on what the Hub can spend, and only its owner can choose them.
 */
export function runnerPolicyCommand(need: InstanceSettingNeed): string {
  switch (need) {
    case 'spend':
      return 'demeteo-runner hub policy --spend on --run-ceiling 2500 --daily-ceiling 10000';
    case 'gates':
      return 'demeteo-runner hub policy --gates on';
    case 'ceiling':
      return 'demeteo-runner hub policy --run-ceiling 2500 --daily-ceiling 10000';
  }
}

const WHAT: Record<InstanceSettingNeed, string> = {
  spend: 'Spend lets the Hub start and cancel Features there. It also needs a per-run and a daily ceiling, or every Feature is refused.',
  gates: 'Gates lets you approve or reject its gates from the Hub.',
  ceiling: 'A ceiling that is not set refuses every Feature, and a run whose max cost is above the per-run ceiling is refused too.',
};

const DESKTOP_STEP: Record<InstanceSettingNeed, string> = {
  spend: 'On that desktop, open Settings › Hub, turn on Spend, and set a run ceiling and a daily ceiling.',
  gates: 'On that desktop, open Settings › Hub and turn on Gates.',
  ceiling: 'On that desktop, open Settings › Hub and set a run ceiling and a daily ceiling.',
};

const ANY_DESKTOP_STEP: Record<InstanceSettingNeed, string> = {
  spend: 'On a desktop, open Settings › Hub, turn on Spend, and set a run ceiling and a daily ceiling.',
  gates: 'On a desktop, open Settings › Hub and turn on Gates.',
  ceiling: 'On a desktop, open Settings › Hub and set a run ceiling and a daily ceiling.',
};

interface InstanceSettingsHelpProps {
  /** `null` when the kind is not known here: both ways are shown. */
  kind: InstanceKind;
  need: InstanceSettingNeed;
}

/**
 * How to turn on what an instance refused for. These switches live only on
 * the instance (docs/HUB.md §7): the Hub can never turn one on, so the most
 * it can do is say exactly where to go.
 */
export function InstanceSettingsHelp({ kind, need }: InstanceSettingsHelpProps) {
  const command = runnerPolicyCommand(need);
  return (
    <div className="nested-card flex flex-col gap-2 px-3 py-3 text-sm text-slate-300">
      <p>{WHAT[need]} Only the instance can turn this on.</p>
      {kind !== 'desktop' && (
        <div className="flex flex-col gap-1.5">
          <p>{kind === null ? 'On a runner, run this on its host:' : 'On the runner’s host, run:'}</p>
          <div className="flex min-w-0 items-center gap-2">
            <code className="min-w-0 flex-1 rounded-md border border-hairline bg-white/5 px-2 py-1.5 font-mono text-xs break-all text-slate-100">
              {command}
            </code>
            <PairingCopyButton text={command} label="Copy the runner command" />
          </div>
          {need !== 'gates' && (
            <p className="text-xs text-slate-400">The ceilings are in cents: pick your own limits.</p>
          )}
          <p className="text-xs text-slate-400">
            Or, from a desktop that manages this runner: Settings › Machines, then the runner’s Hub panel.
          </p>
        </div>
      )}
      {kind !== 'runner' && <p>{kind === null ? ANY_DESKTOP_STEP[need] : DESKTOP_STEP[need]}</p>}
    </div>
  );
}
