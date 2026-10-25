import { PlaceholderCard } from '../components/PlaceholderCard';

/**
 * Desktop pairing only. The design's second path — a runner printing a code
 * to type here — is not built: runners are not paired with the Hub, so adding
 * a field or a tab for it would offer a flow that cannot finish.
 */
export function AddInstance() {
  return (
    <PlaceholderCard title="Add instance">
      <p>
        Pairing starts on the desktop, not here. Your browser opens this Hub for you to approve it.
      </p>
      <ol className="mt-4 space-y-3">
        <li className="nested-card flex gap-3 px-4 py-3">
          <span aria-hidden="true" className="font-heading font-bold text-violet-400">1</span>
          <span>Open Settings › Hub in Demeteo on the desktop you want to add.</span>
        </li>
        <li className="nested-card flex gap-3 px-4 py-3">
          <span aria-hidden="true" className="font-heading font-bold text-violet-400">2</span>
          <span>
            Enter this Hub's URL:{' '}
            <span className="font-mono text-slate-200 break-all">{window.location.origin}</span>
          </span>
        </li>
        <li className="nested-card flex gap-3 px-4 py-3">
          <span aria-hidden="true" className="font-heading font-bold text-violet-400">3</span>
          <span>Click Connect, then compare and approve the fingerprint in the browser page that opens.</span>
        </li>
      </ol>
    </PlaceholderCard>
  );
}
