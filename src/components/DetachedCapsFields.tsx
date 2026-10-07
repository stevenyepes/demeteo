import type { DetachedCap } from '../lib/detachedCaps';

interface DetachedCapsFieldsProps {
  maxCostUsd: string;
  onMaxCostUsdChange: (raw: string) => void;
  costCap: DetachedCap;
  maxWallClockMins: string;
  onMaxWallClockMinsChange: (raw: string) => void;
  wallClockCap: DetachedCap;
}

/** The detached run's optional cost and wall-clock caps. Parsing stays with
 *  the caller, which also gates Launch on the same `DetachedCap` values. */
export function DetachedCapsFields({
  maxCostUsd,
  onMaxCostUsdChange,
  costCap,
  maxWallClockMins,
  onMaxWallClockMinsChange,
  wallClockCap,
}: DetachedCapsFieldsProps) {
  return (
    <div className="grid grid-cols-2 gap-2">
      <div>
        <label
          htmlFor="start-feature-max-cost"
          className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider"
        >
          Max cost (USD)
        </label>
        <input
          id="start-feature-max-cost"
          type="text"
          inputMode="decimal"
          value={maxCostUsd}
          onChange={(e) => onMaxCostUsdChange(e.target.value)}
          placeholder="no cap"
          aria-invalid={costCap.error !== null}
          aria-describedby={costCap.error ? 'start-feature-max-cost-error' : undefined}
          className="w-full bg-[var(--bg-input)] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
        />
        {costCap.error && (
          <p id="start-feature-max-cost-error" className="mt-1 text-[10px] font-mono text-ruby-300">
            {costCap.error}
          </p>
        )}
      </div>
      <div>
        <label
          htmlFor="start-feature-max-wall-clock"
          className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider"
        >
          Max wall-clock (min)
        </label>
        <input
          id="start-feature-max-wall-clock"
          type="text"
          inputMode="numeric"
          value={maxWallClockMins}
          onChange={(e) => onMaxWallClockMinsChange(e.target.value)}
          placeholder="no cap"
          aria-invalid={wallClockCap.error !== null}
          aria-describedby={wallClockCap.error ? 'start-feature-max-wall-clock-error' : undefined}
          className="w-full bg-[var(--bg-input)] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
        />
        {wallClockCap.error && (
          <p id="start-feature-max-wall-clock-error" className="mt-1 text-[10px] font-mono text-ruby-300">
            {wallClockCap.error}
          </p>
        )}
      </div>
    </div>
  );
}
