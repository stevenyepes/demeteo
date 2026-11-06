import { PageHeader } from '../components/PageHeader';
import { PairingCopyButton } from '../components/PairingCopyButton';
import { PairingStep as Step } from '../components/PairingStep';
import { routeHref } from '../lib/router';

/**
 * Both paths start on the machine, never here: the Hub cannot reach an
 * instance, so the page that adds one only explains what to run where. A
 * runner's code is typed on `#/device`, not in a field on this page, so that
 * the code a runner prints as a link and the code a human types land on the
 * same lookup.
 */
export function AddInstance() {
  const origin = window.location.origin;
  const command = `demeteo-runner hub pair ${origin}`;
  return (
    <div className="flex flex-col gap-7">
      <PageHeader
        title="Add instance"
        eyebrow={
          <a href={routeHref({ name: 'fleet' })} className="transition-colors hover:text-white">
            Fleet
          </a>
        }
      />
      <p className="-mt-3 max-w-2xl text-sm text-slate-400">
        Pairing starts on the machine you want to add, not here. You approve it on this Hub.
      </p>
      <div className="flex flex-wrap items-start gap-6">
        <section aria-labelledby="add-instance-desktop" className="panel min-w-0 flex-[1_1_420px] p-6">
          <h2 id="add-instance-desktop" className="mb-5 font-heading text-base font-semibold text-white">
            A desktop
          </h2>
          <ol aria-labelledby="add-instance-desktop" className="flex flex-col gap-6">
            <Step number={1} title="In Demeteo, open Settings › Hub">
              On the desktop you want to add.
            </Step>
            <Step number={2} title="Enter this Hub's URL">
              <div className="mt-1 flex items-center gap-3 rounded-md border border-hairline bg-surface-well py-1.5 pr-1.5 pl-3">
                <span className="min-w-0 flex-1 font-mono text-sm text-slate-200 break-all">{origin}</span>
                <PairingCopyButton text={origin} label="Copy this Hub's URL" />
              </div>
            </Step>
            <Step number={3} title="Click Connect, then approve it here">
              The browser opens on this Hub. Compare the fingerprint with the one Demeteo shows,
              then choose which scopes to grant.
            </Step>
          </ol>
        </section>
        <section aria-labelledby="add-instance-runner" className="panel min-w-0 flex-[1_1_420px] p-6">
          <h2 id="add-instance-runner" className="mb-5 font-heading text-base font-semibold text-white">
            A runner host
          </h2>
          <ol aria-labelledby="add-instance-runner" className="flex flex-col gap-6">
            <Step number={1} title="On the host, run">
              <div className="mt-1 flex items-center gap-3 rounded-md border border-hairline bg-surface-well py-1.5 pr-1.5 pl-3">
                <span className="min-w-0 flex-1 font-mono text-sm text-slate-200 break-all">{command}</span>
                <PairingCopyButton text={command} label="Copy the pairing command" />
              </div>
            </Step>
            <Step number={2} title="Open the link it prints, or enter its code">
              The runner prints a short code, a link to this Hub and its fingerprint. Enter the code
              on <a href={routeHref({ name: 'device' })} className="text-accent-ink hover:underline">Pair runner</a>.
            </Step>
            <Step number={3} title="Compare the fingerprint, then approve with your passkey">
              The code is good for ten minutes. The runner finishes pairing on its own once you
              approve.
            </Step>
          </ol>
        </section>
        <section aria-labelledby="add-instance-trust" className="panel min-w-0 flex-[1_1_340px] p-6">
          <h2 id="add-instance-trust" className="font-heading text-base font-semibold text-white">
            What pairing changes
          </h2>
          <div className="mt-4 flex flex-col gap-3 text-sm text-slate-300">
            <p>The machine opens one outbound connection to the Hub. The Hub never connects to it.</p>
            <p>
              Its device key is created on the machine: in the OS keyring on a desktop, sealed with
              systemd-creds on a runner host. Only the public half leaves it.
            </p>
            <p>
              Provider keys, Git tokens, transcripts and diffs never leave it. Each scope you grant
              can be turned off again on the machine itself.
            </p>
          </div>
        </section>
      </div>
    </div>
  );
}
