import { PageHeader } from '../components/PageHeader';
import { PairingCopyButton } from '../components/PairingCopyButton';
import { PairingStep as Step } from '../components/PairingStep';
import { routeHref } from '../lib/router';

/**
 * Desktop pairing only. The design's second path — a runner printing a code
 * to type here — is not built: runners are not paired with the Hub, so adding
 * a field or a tab for it would offer a flow that cannot finish.
 */
export function AddInstance() {
  const origin = window.location.origin;
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
        Pairing starts on the desktop, not here. Your browser opens this Hub for you to approve it.
      </p>
      <div className="flex flex-wrap items-start gap-6">
        <section aria-label="Steps" className="panel min-w-0 flex-[1_1_420px] p-6">
          <ol className="flex flex-col gap-6">
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
        <section aria-labelledby="add-instance-trust" className="panel min-w-0 flex-[1_1_340px] p-6">
          <h2 id="add-instance-trust" className="font-heading text-base font-semibold text-white">
            What pairing changes
          </h2>
          <div className="mt-4 flex flex-col gap-3 text-sm text-slate-300">
            <p>The desktop opens one outbound connection to the Hub. The Hub never connects to it.</p>
            <p>Its device key is created in the OS keyring. Only the public half leaves the machine.</p>
            <p>
              Provider keys, Git tokens, transcripts and diffs never leave it. Each scope you grant
              can be turned off again in Settings › Hub.
            </p>
          </div>
        </section>
      </div>
    </div>
  );
}
