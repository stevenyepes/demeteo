import type { CreateProjectStepPayload } from '../../types';

export interface NameStepProps {
  /** Current (pre-submit) name value. Empty string = user hasn't
   *  typed yet. */
  value: string;
  /** Called when the user advances. The wizard validates server-side
   *  via `port.validate_name` (see Rust), but the local hook also
   *  blocks empty submissions so the Next button stays disabled. */
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'name' }>) => void;
}

/**
 * Step 1 — Name. Single text input where the user types the
 * repository slug they want created. The slug is also used as the
 * display name and (by default) the new repo's `name` on the
 * provider. Validation rules mirror the Rust
 * `CreateProjectPort::validate_name` (lowercase alphanumeric, dots,
 * dashes, underscores; 1–100 chars; must start with alphanumeric).
 */
export function NameStep({ value, onSubmit }: NameStepProps) {
  const trimmed = value.trim();
  const tooShort = trimmed.length < 2;
  const invalid = !SLUG_TEST.test(trimmed);

  return (
    <div className="space-y-3" data-testid="wizard-step-name">
      <label
        htmlFor="wizard-name-input"
        className="text-[11px] font-mono text-slate-400 uppercase tracking-widest"
      >
        What do you want to call this project?
      </label>
      <input
        id="wizard-name-input"
        type="text"
        value={value}
        onChange={(e) => onSubmit({ step: 'name', value: e.target.value })}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !tooShort && !invalid) {
            e.preventDefault();
            onSubmit({ step: 'name', value: trimmed });
          }
        }}
        placeholder="e.g. billing-service-rust"
        autoFocus
        aria-invalid={value.trim().length > 0 && (tooShort || invalid)}
        className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white placeholder-slate-600 focus:outline-none focus:border-violet-500/50"
      />
      <p className="text-[11px] text-slate-500 font-mono">
        Lowercase letters, digits, dots, dashes or underscores. Used
        as the new repo's slug and the project's display name.
      </p>
      {value.trim().length > 0 && (tooShort || invalid) && (
        <p className="text-[11px] text-amber-300 font-mono" data-testid="wizard-name-error">
          {tooShort ? 'Use at least 2 characters' : 'Slug must be lowercase and contain only letters, digits, dots, dashes or underscores'}
        </p>
      )}
    </div>
  );
}

// Same rule as Rust `SLUG_PATTERN` (see `src-tauri/src/ports/create_project_port.rs`).
// Mirrored here so the input can show inline validation without
// having to round-trip through the IPC.
const SLUG_TEST = /^[a-z0-9][a-z0-9._-]{0,99}$/;