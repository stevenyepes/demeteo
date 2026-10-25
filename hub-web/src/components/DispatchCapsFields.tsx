import { useId } from 'react';

interface DispatchCapsFieldsProps {
  maxCostUsd: string;
  onMaxCostUsdChange: (raw: string) => void;
  costError?: string | null;
  maxWallClockMins: string;
  onMaxWallClockMinsChange: (raw: string) => void;
  wallClockError?: string | null;
}

/**
 * Not `src/components/DetachedCapsFields.tsx`: its copy says blank means no
 * cap, which is true here for wall-clock only. Parsing stays with the caller
 * (`lib/dispatchForm.ts`), which hands back the error strings shown here.
 *
 * Required is `aria-required`, not `required`: the native constraint would
 * block the submit event before the form can show its own field error.
 */
export function DispatchCapsFields({
  maxCostUsd,
  onMaxCostUsdChange,
  costError,
  maxWallClockMins,
  onMaxWallClockMinsChange,
  wallClockError,
}: DispatchCapsFieldsProps) {
  const costId = useId();
  const wallClockId = useId();

  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <div className="flex flex-col gap-1.5">
        <div className="flex items-baseline gap-2">
          <label htmlFor={costId} className="text-xs font-medium text-slate-300">
            Max cost (USD)
          </label>
          <span aria-hidden="true" className="text-xs text-violet-300">
            required
          </span>
        </div>
        <input
          id={costId}
          type="text"
          inputMode="decimal"
          value={maxCostUsd}
          onChange={(event) => onMaxCostUsdChange(event.target.value)}
          autoComplete="off"
          aria-required="true"
          aria-invalid={Boolean(costError)}
          aria-describedby={costError ? `${costId}-help ${costId}-error` : `${costId}-help`}
          className="input-field font-mono focus:border-cyan-400! focus:shadow-none! focus-visible:outline-2 focus-visible:outline-cyan-400"
        />
        <p id={`${costId}-help`} className="text-xs text-slate-500">
          The instance refuses to start a run with no cap, or with a cap above its own local ceiling, which the Hub
          cannot see.
        </p>
        {costError && (
          <p id={`${costId}-error`} className="text-xs text-ruby-300">
            {costError}
          </p>
        )}
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={wallClockId} className="text-xs font-medium text-slate-300">
          Max wall-clock (min)
        </label>
        <input
          id={wallClockId}
          type="text"
          inputMode="numeric"
          value={maxWallClockMins}
          onChange={(event) => onMaxWallClockMinsChange(event.target.value)}
          autoComplete="off"
          aria-invalid={Boolean(wallClockError)}
          aria-describedby={wallClockError ? `${wallClockId}-help ${wallClockId}-error` : `${wallClockId}-help`}
          className="input-field font-mono focus:border-cyan-400! focus:shadow-none! focus-visible:outline-2 focus-visible:outline-cyan-400"
        />
        <p id={`${wallClockId}-help`} className="text-xs text-slate-500">
          Optional. Leave blank for no wall-clock cap.
        </p>
        {wallClockError && (
          <p id={`${wallClockId}-error`} className="text-xs text-ruby-300">
            {wallClockError}
          </p>
        )}
      </div>
    </div>
  );
}
