import { useEffect, useState, type ReactNode } from 'react';
import { ArrowLeft, Check, ChevronLeft } from 'lucide-react';
import { BootstrapStep, STEP_ORDER } from '../../types';

// ── Layout tokens (AGENTS.md §5) ─────────────────────────────────────────
// Background:    #08090c / #0d0f14 — app shell
// Card surface:  rgba(18,22,30,0.75) — glassmorphism
// Border glow:   rgba(255,255,255,0.05)
// Violet:        #8b5cf6 — primary action / active step
// Cyan:          #06b6d4 — interactive / next-step
// Emerald:       #10b981 — completed steps
// Ruby:          #ef4444 — errors / blocked

export interface WizardShellProps {
  /** Current wizard step (mirrors `BootstrapState.step`). */
  step: BootstrapStep;
  /** Full wizard history — drives both the progress dots (via
   *  `isCompleted` derivation) and the `goBack` enablement check.
   *  **Never** derive the active step from an index into
   *  {@link STEP_ORDER}; doing so silently re-enters auto-progressed
   *  screens. */
  history: ReadonlyArray<BootstrapStep>;
  /** Short reason text shown next to the disabled Next button. */
  reason: string;
  /** True when the user is allowed to advance. When false the Next
   *  button is disabled and `reason` is surfaced inline. */
  canProceed: boolean;
  /** Handler invoked when the user clicks Back. Should call the
   *  Rust `go_back_create_project` IPC and replace the wizard's
   *  state with the returned (already-rewound) state — see
   *  `CreateProjectWizard`. */
  onBack: () => void;
  /** Handler invoked when the user clicks Next. */
  onNext: () => void;
  /** Label of the primary CTA. Defaults to "Next" / "Create
   *  project" on the final Description step. */
  nextLabel?: string;
  /** True when the wizard is on its terminal screen and Next should
   *  commit instead of advancing. */
  isFinal?: boolean;
  /** Active step's body — the wizard renders exactly one decision
   *  surface here at a time. */
  children: ReactNode;
}

/** Display labels for the seven progress dots. Stable across renders
 *  so the AnimatePresence-style transition only swaps the active
 *  highlight, not the whole row. */
const STEP_LABELS: Record<BootstrapStep, string> = {
  name: 'Name',
  provider: 'Provider',
  group: 'Group',
  machine: 'Machine',
  agent: 'Agent',
  model: 'Model',
  description: 'Description',
};

/** Fade-in keyframe used for step transitions. The CSS class
 *  `.animate-fade-in` lives in `src/App.css` (declared next to the
 *  rest of the design-system animations). Returning it from a
 *  derived `transitionKey` lets React re-mount the children on every
 *  step change, retriggering the animation. */
function useStepTransitionKey(step: BootstrapStep): string {
  const [tick, setTick] = useState(0);
  useEffect(() => { setTick((t) => t + 1); }, [step]);
  return `${step}-${tick}`;
}

/**
 * The shared shell that wraps each of the seven wizard step
 * components. Renders:
 *
 * - A glassmorphism card (per AGENTS.md §5) containing the active
 *   step's body.
 * - A horizontal seven-dot progress indicator with per-dot labels,
 *   where completed dots glow emerald, the active one glows violet,
 *   and pending ones stay muted.
 * - A Back / Next footer with a disabled Back when
 *   `history.length <= 1`.
 *
 * The shell owns no wizard state — it is a pure presentation layer
 * over the props supplied by `CreateProjectWizard`. In particular,
 * the **`goBack` enablement is derived from `history.length <= 1`**
 * (matching the Rust `BootstrapState::can_go_back`), so an
 * auto-progressed step in `history` still counts toward the
 * disable threshold. The wizard's reducer is therefore the single
 * source of truth for "can the user go back?".
 */
export function WizardShell(props: WizardShellProps) {
  const { step, history, reason, canProceed, onBack, onNext,
    nextLabel, isFinal = false, children } = props;

  const transitionKey = useStepTransitionKey(step);
  const canGoBack = history.length > 1;

  return (
    <div
      className="flex-1 overflow-y-auto p-6 relative flex items-center justify-center bg-[#08090c]"
      data-wizard-step={step}
    >
      {/* Decorative violet glow — mirrors the existing wizard chrome
          so the surface feels visually consistent. */}
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[800px] h-[400px] bg-violet-600/10 rounded-full blur-[120px] pointer-events-none" />

      <div className="w-full max-w-3xl z-10 space-y-6">
        <header className="flex items-center gap-3">
          <span className="w-2 h-2 rounded-full bg-violet-400 shadow-[0_0_12px_rgba(139,92,246,0.6)]" />
          <div>
            <h1 className="text-2xl font-outfit font-bold text-white">Create a project</h1>
            <p className="text-xs text-slate-400">
              A guided, one-decision-at-a-time setup. Seven steps.
            </p>
          </div>
        </header>

        {/* Glassmorphism card per AGENTS.md §5. Backdrop-blur + the
            documented surface colour give the wizard its signature
            frosted look. */}
        <div
          className="glass-panel p-6 rounded-2xl border border-white/5 shadow-2xl space-y-6"
          style={{
            background: 'rgba(18,22,30,0.75)',
            backdropFilter: 'blur(12px)',
            WebkitBackdropFilter: 'blur(12px)',
          }}
        >
          <StepDots step={step} history={history} />

          {/* Step body. The `key` swap is what drives the
              AnimatePresence-style fade: on every step change the
              wrapper re-mounts and replays `animate-fade-in`. */}
          <div key={transitionKey} className="animate-fade-in">
            {children}
          </div>

          <div className="flex items-center justify-between pt-2 border-t border-white/5">
            <button
              type="button"
              onClick={onBack}
              disabled={!canGoBack}
              data-testid="wizard-back"
              className="px-4 py-2 text-xs font-medium text-slate-400 hover:text-white transition-colors disabled:opacity-30 disabled:cursor-not-allowed flex items-center gap-1.5"
            >
              <ChevronLeft className="w-4 h-4" /> Back
            </button>
            <div className="flex items-center gap-3">
              <span className="text-[10px] text-slate-500 font-mono" data-testid="wizard-reason">
                {reason || (isFinal ? 'Launch the feature' : 'Ready')}
              </span>
              <button
                type="button"
                onClick={onNext}
                disabled={!canProceed}
                data-testid="wizard-next"
                className={`px-5 py-2 text-xs font-bold rounded-lg transition-all flex items-center gap-1.5 ${
                  canProceed
                    ? isFinal
                      ? 'bg-emerald-600 hover:bg-emerald-500 text-white shadow-[0_0_15px_rgba(16,185,129,0.3)]'
                      : 'bg-cyan-600 hover:bg-cyan-500 text-white shadow-[0_0_15px_rgba(6,182,212,0.3)]'
                    : 'bg-white/5 text-slate-600 cursor-not-allowed'
                }`}
              >
                {nextLabel ?? (isFinal ? 'Create project' : 'Next')}
                {!isFinal && <ArrowLeft className="w-3.5 h-3.5 rotate-180" />}
              </button>
            </div>
          </div>
        </div>

        <div className="flex items-center justify-between text-[10px] text-slate-500 font-mono">
          <span>Step {STEP_ORDER.indexOf(step) + 1} of {STEP_ORDER.length}</span>
          <span className="text-slate-600">history: {history.length}</span>
        </div>
      </div>
    </div>
  );
}

// ── Progress dots ──────────────────────────────────────────────────────

interface StepDotsProps {
  step: BootstrapStep;
  history: ReadonlyArray<BootstrapStep>;
}

/** Seven-dot progress indicator. A dot is "completed" iff it appears
 *  in `history` strictly before the current step; the active dot
 *  glows violet; pending dots stay muted. This mirrors the wizard's
 *  Rust state machine, where `history` records every step the wizard
 *  transitioned to (including auto-progressed ones). */
function StepDots({ step, history }: StepDotsProps) {
  const activeIdx = STEP_ORDER.indexOf(step);
  const completed = new Set(history.slice(0, history.indexOf(step)));
  return (
    <div className="w-full overflow-x-auto" data-testid="wizard-dots">
      <ol className="flex items-center gap-2 min-w-fit py-1" aria-label="Wizard progress">
        {STEP_ORDER.map((s, idx) => {
          const isActive = s === step;
          const isDone = completed.has(s) || (activeIdx >= 0 && idx < activeIdx && history.includes(s));
          const label = STEP_LABELS[s];
          return (
            <li
              key={s}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-full border text-[11px] font-mono uppercase tracking-wider transition-all duration-300 shrink-0 ${
                isActive
                  ? 'bg-violet-500/10 border-violet-500/50 text-violet-200 shadow-[0_0_15px_rgba(139,92,246,0.25)]'
                  : isDone
                    ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-200'
                    : 'bg-black/30 border-white/10 text-slate-500'
              }`}
              aria-current={isActive ? 'step' : undefined}
            >
              <span
                className={`w-5 h-5 rounded-full flex items-center justify-center text-[10px] font-semibold shrink-0 ${
                  isActive
                    ? 'bg-violet-500/30 text-violet-100'
                    : isDone
                      ? 'bg-emerald-500/30 text-emerald-100'
                      : 'bg-white/5 text-slate-500'
                }`}
              >
                {isDone ? <Check className="w-3 h-3" /> : idx + 1}
              </span>
              <span className="whitespace-nowrap">{label}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}

// ── Pure helpers (exported for unit tests) ─────────────────────────────

/** True iff the wizard's history allows a backward step. Mirrors
 *  Rust `BootstrapState::can_go_back`: returns true exactly when
 *  there is at least one entry to pop. */
export function canGoBackFromHistory(history: ReadonlyArray<BootstrapStep>): boolean {
  return history.length > 1;
}

/** Pure derivation: the step a single `goBack` should land on. Mirrors
 *  the Rust `BootstrapState::go_back` pop semantics — never subtracts
 *  one from an index into STEP_ORDER. Returns `null` when there is
 *  no step to rewind to. */
export function rewindHistory(
  history: ReadonlyArray<BootstrapStep>,
): BootstrapStep | null {
  if (history.length <= 1) return null;
  return history[history.length - 2] ?? null;
}