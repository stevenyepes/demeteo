import { useEffect, useState } from 'react';
import { CircleDollarSign } from 'lucide-react';
import type { HubLocalPolicy } from '../../lib/hub';
import { UNSET_CEILING_COPY, ceilingInputValue, parseCeilingDollars } from '../../lib/hubPresentation';

export type CeilingField = 'run_ceiling_cents' | 'daily_ceiling_cents';

const CEILINGS: ReadonlyArray<{ field: CeilingField; label: string; description: string }> = [
  {
    field: 'run_ceiling_cents',
    label: 'Per-run maximum',
    description: 'The most one Hub-started run may cost.',
  },
  {
    field: 'daily_ceiling_cents',
    label: 'Rolling 24-hour total',
    description: 'The most all Hub-started runs may cost together in any 24 hours.',
  },
];

type Drafts = Record<CeilingField, string>;
type Errors = Record<CeilingField, string | null>;

const NO_ERRORS: Errors = { run_ceiling_cents: null, daily_ceiling_cents: null };

interface HubCeilingsCardProps {
  policy: HubLocalPolicy | null;
  saving: boolean;
  onSave: (field: CeilingField, cents: number | null) => void;
}

export function HubCeilingsCard({ policy, saving, onSave }: HubCeilingsCardProps) {
  const run = policy?.run_ceiling_cents ?? null;
  const daily = policy?.daily_ceiling_cents ?? null;
  const [drafts, setDrafts] = useState<Drafts>({
    run_ceiling_cents: ceilingInputValue(run),
    daily_ceiling_cents: ceilingInputValue(daily),
  });
  const [errors, setErrors] = useState<Errors>(NO_ERRORS);

  useEffect(() => {
    setDrafts({ run_ceiling_cents: ceilingInputValue(run), daily_ceiling_cents: ceilingInputValue(daily) });
    setErrors(NO_ERRORS);
  }, [run, daily]);

  const commit = (field: CeilingField) => {
    if (!policy || saving) return;
    const parsed = parseCeilingDollars(drafts[field]);
    if (!parsed.ok) {
      setErrors((current) => ({ ...current, [field]: parsed.error }));
      return;
    }
    setErrors((current) => ({ ...current, [field]: null }));
    if (parsed.cents !== policy[field]) onSave(field, parsed.cents);
  };

  return (
    <div className="glass-panel p-6 rounded-xl space-y-4">
      <h3 className="font-heading text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
        <CircleDollarSign className="w-4 h-4 text-violet-400" /> Spending ceilings
      </h3>
      <p className="text-xs text-slate-400 leading-relaxed">
        A Hub start over either ceiling is refused on this machine. Leave one blank and every Hub
        start is refused; a ceiling of 0 refuses every start that costs anything.
      </p>
      <div className="space-y-2">
        {CEILINGS.map(({ field, label, description }) => {
          const error = errors[field];
          const inputId = `hub-ceiling-${field}`;
          return (
            <form
              key={field}
              className="nested-card p-4 space-y-2"
              onSubmit={(e) => {
                e.preventDefault();
                commit(field);
              }}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0 space-y-1">
                  <label htmlFor={inputId} className="block text-sm text-slate-200">
                    {label}
                  </label>
                  <p className="text-xs text-slate-400 leading-relaxed">{description}</p>
                </div>
                <div className="flex items-center gap-1 shrink-0">
                  <span className="font-mono text-sm text-slate-400">$</span>
                  <input
                    id={inputId}
                    type="text"
                    inputMode="decimal"
                    value={drafts[field]}
                    onChange={(e) => setDrafts((current) => ({ ...current, [field]: e.target.value }))}
                    onBlur={() => commit(field)}
                    disabled={!policy || saving}
                    aria-invalid={error !== null}
                    aria-describedby={`${inputId}-note`}
                    placeholder="Not set"
                    className={`w-28 bg-black/40 border rounded-lg py-2 px-3 text-sm text-white font-mono text-right focus:outline-none focus:border-cyan-500/50 placeholder-slate-600 disabled:opacity-50 ${
                      error ? 'border-ruby-500/50' : 'border-slate-700/60'
                    }`}
                  />
                </div>
              </div>
              <div id={`${inputId}-note`}>
                {error ? (
                  <p className="text-xs text-ruby-400">{error}</p>
                ) : (
                  policy?.[field] === null && <p className="text-xs text-amber-400">{UNSET_CEILING_COPY}</p>
                )}
              </div>
            </form>
          );
        })}
      </div>
    </div>
  );
}

export default HubCeilingsCard;
