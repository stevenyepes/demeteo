import type { CreateProjectStepPayload } from '../../types';

export interface DescriptionStepProps {
  /** The user's free-text description. */
  description: string;
  /** Suggested feature title (defaults to the project slug, but the
   *  user can override it inline). */
  title: string;
  /** Repo visibility, used to build the commit payload's
   *  `visibility` field. */
  visibility: 'private' | 'public';
  onSubmit: (payload: Extract<CreateProjectStepPayload, { step: 'commit' }>) => void;
}

/**
 * Step 7 — Description. Final screen before the wizard commits.
 * Combines the free-text "what do you want to build?" prompt with
 * the optional title override and a small public/private toggle so
 * the commit payload carries every field the Rust
 * `submit_create_project_step` needs to launch the feature.
 *
 * The component renders, but does not submit, until both the title
 * and the description pass the minimum-length gate. The orchestrator
 * disables the Next button while either is too short; the inline
 * error message surfaces the specific gate that is failing.
 */
export function DescriptionStep({
  description, title, visibility, onSubmit,
}: DescriptionStepProps) {
  const titleTooShort = title.trim().length < 1;
  const descriptionTooShort = description.trim().length < 8;
  const showTitleError = titleTooShort && title.length > 0;
  const showDescError = descriptionTooShort && description.length > 0;

  const ready = !titleTooShort && !descriptionTooShort;

  return (
    <div className="space-y-4" data-testid="wizard-step-description">
      <div>
        <label
          htmlFor="wizard-title"
          className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block mb-1"
        >
          Feature title
        </label>
        <input
          id="wizard-title"
          type="text"
          value={title}
          onChange={(e) => onSubmit(commitPayloadWith({ title: e.target.value }))}
          placeholder="e.g. Implement billing-service in Rust"
          className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50"
        />
        {showTitleError && (
          <p className="mt-1 text-[11px] text-amber-300 font-mono">
            Title cannot be empty.
          </p>
        )}
      </div>

      <div>
        <label
          htmlFor="wizard-description"
          className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block mb-1"
        >
          What do you want to build?
        </label>
        <textarea
          id="wizard-description"
          value={description}
          onChange={(e) => onSubmit(commitPayloadWith({ description: e.target.value }))}
          rows={8}
          placeholder="Describe the feature. The pipeline will research, draft a spec, implement, review, validate, and ship."
          className="w-full bg-black/40 border border-white/10 rounded-lg p-3 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 resize-y"
        />
        <p className="mt-1 text-[10px] text-slate-500 font-mono">
          {description.trim().length} characters · minimum 8.
        </p>
        {showDescError && (
          <p className="mt-1 text-[11px] text-amber-300 font-mono">
            Describe the feature in a sentence or two (8+ characters).
          </p>
        )}
      </div>

      <div>
        <label className="text-[11px] font-mono text-slate-400 uppercase tracking-widest block mb-1">
          Repo visibility
        </label>
        <div className="grid grid-cols-2 gap-1.5">
          <button
            type="button"
            onClick={() => onSubmit(commitPayloadWith({ visibility: 'private' }))}
            data-testid="wizard-visibility-private"
            className={`px-3 py-2 rounded-lg text-[11px] font-mono border transition-all ${
              visibility === 'private'
                ? 'bg-violet-500/10 border-violet-500/50 text-violet-200'
                : 'bg-black/30 border-white/10 text-slate-400 hover:border-white/20'
            }`}
          >
            Private
          </button>
          <button
            type="button"
            onClick={() => onSubmit(commitPayloadWith({ visibility: 'public' }))}
            data-testid="wizard-visibility-public"
            className={`px-3 py-2 rounded-lg text-[11px] font-mono border transition-all ${
              visibility === 'public'
                ? 'bg-cyan-500/10 border-cyan-500/50 text-cyan-200'
                : 'bg-black/30 border-white/10 text-slate-400 hover:border-white/20'
            }`}
          >
            Public
          </button>
        </div>
      </div>

      {/* Hidden input that exposes the readiness gate to the parent
          so the wizard can render an inline progress hint. The
          status string is also surfaced via `data-ready` so the
          orchestrator can assert against it in tests. */}
      <p className="text-[10px] text-slate-500 font-mono" data-ready={ready}>
        {ready
          ? `Ready to launch ${visibility} repo + Standard pipeline.`
          : 'Complete the title and description to launch.'}
      </p>
    </div>
  );
}

// Local helper used by the description / title / visibility inputs
// to emit *partial* commit payloads. The orchestrator merges these
// patches into the full commit snapshot at submit time (see
// `CreateProjectWizard.commitPayload`).
function commitPayloadWith(
  patch: Partial<Pick<Extract<CreateProjectStepPayload, { step: 'commit' }>,
    'title' | 'description' | 'visibility'>>,
): Extract<CreateProjectStepPayload, { step: 'commit' }> {
  // The orchestrator-supplied defaults live in the React state; we
  // emit a sentinel payload carrying only the patched fields and
  // the wizard reducer fills the rest. This keeps the step
  // component decoupled from the broader commit payload.
  return {
    step: 'commit',
    title: patch.title ?? '',
    description: patch.description ?? '',
    visibility: patch.visibility ?? 'private',
    // Placeholders — the wizard reducer replaces these at submit.
    name: '',
    providerId: '',
    providerKind: '',
    providerHost: '',
    namespaceId: '',
    namespaceKind: '',
    namespaceName: '',
    machineKind: 'local',
    machineId: null,
    agentKind: '',
    model: '',
  };
}