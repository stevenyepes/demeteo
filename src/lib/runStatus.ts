/**
 * Canonical status vocabulary for runs (ux-audit F27). One mapping from
 * every status string a run surface can produce — local `Feature.status`,
 * the runner's `RunnerRun.status` as mirrored into `RemoteRunMirror`, and
 * the laptop-only `unreachable` — to a display label + tone, so every run
 * surface (FeatureDetail, ProjectHome) speaks the same color language
 * instead of each view keeping its own ternary chain.
 *
 * Tone semantics (docs/UX_JOURNEYS.md §2, as settled by F27):
 *   cyan    = in motion (running)
 *   violet  = in motion, agent-judged (verifying)
 *   amber   = needs a human (gates, credentials, interruptions)
 *   emerald = done well (completed / PR ready)
 *   ruby    = done badly (failed / over budget)
 *   slate   = inert (queued, cancelled, unreachable, skipped)
 */

export type RunStatusTone = 'emerald' | 'cyan' | 'violet' | 'amber' | 'ruby' | 'slate';

export interface RunStatusMeta {
  label: string;
  tone: RunStatusTone;
  /** Still changing on its own — worth a pulse/live affordance. */
  active: boolean;
}

const META: Record<string, RunStatusMeta> = {
  pending:              { label: 'Queued',            tone: 'slate',   active: true },
  bootstrapping:        { label: 'Bootstrapping',     tone: 'amber',   active: true },
  running:              { label: 'Running',           tone: 'cyan',    active: true },
  verifying:            { label: 'Verifying',         tone: 'violet',  active: true },
  gated:                { label: 'Gate needs you',    tone: 'amber',   active: false },
  awaiting_gate:        { label: 'Gate needs you',    tone: 'amber',   active: false },
  parked:               { label: 'Gate needs you',    tone: 'amber',   active: false },
  'needs-credentials':  { label: 'Needs credentials', tone: 'amber',   active: false },
  needs_credentials:    { label: 'Needs credentials', tone: 'amber',   active: false },
  'over-budget':        { label: 'Over budget',       tone: 'ruby',    active: false },
  awaiting_mr:          { label: 'PR ready',          tone: 'emerald', active: false },
  pr_ready:             { label: 'PR ready',          tone: 'emerald', active: false },
  published:            { label: 'Published',         tone: 'emerald', active: false },
  completed:            { label: 'Completed',         tone: 'emerald', active: false },
  failed:               { label: 'Failed',            tone: 'ruby',    active: false },
  error:                { label: 'Failed',            tone: 'ruby',    active: false },
  interrupted:          { label: 'Interrupted',       tone: 'amber',   active: false },
  cancelled:            { label: 'Cancelled',         tone: 'slate',   active: false },
  unreachable:          { label: 'Unreachable',       tone: 'slate',   active: false },
};

export function runStatusMeta(status: string): RunStatusMeta {
  return (
    META[status.toLowerCase()] ?? {
      label: status.replace(/[_-]/g, ' '),
      tone: 'slate',
      active: false,
    }
  );
}

/** The fields a run surface needs to resolve a display status. */
export interface FeatureRunStatusFields {
  status: string;
  mr_url?: string | null;
  mr_state?: string | null;
}

/** MR states that mean the run's work actually reached a provider. */
const PUBLISHED_MR_STATES = ['draft', 'open', 'merged'];

/**
 * Display status for a feature, which is not always `Feature.status`.
 *
 * `MrPublisher` collapses a published run to `status = 'completed'` and
 * records the PR on `mr_url`/`mr_state` in the same write, so `status`
 * alone cannot tell "finished, nothing shipped" from "finished, PR is
 * up". Fold that back into one status string here so every surface
 * applies the published-beats-completed rule the same way.
 *
 * `mr_state = 'closed'` (PR closed without merge) deliberately falls
 * through to the feature's own status — nothing was published.
 */
export function featureRunStatus(feature: FeatureRunStatusFields): string {
  if (feature.mr_url && PUBLISHED_MR_STATES.includes((feature.mr_state ?? '').toLowerCase())) {
    return 'published';
  }
  return feature.status;
}

/** `bg/text/border` classes for a pill/chip in the given tone. */
export const TONE_CHIP: Record<RunStatusTone, string> = {
  emerald: 'bg-emerald-500/10 text-emerald-400 border-emerald-500/20',
  cyan:    'bg-cyan-500/10 text-cyan-400 border-cyan-500/20',
  violet:  'bg-violet-500/10 text-violet-400 border-violet-500/20',
  amber:   'bg-amber-500/10 text-amber-400 border-amber-500/20',
  ruby:    'bg-ruby-500/10 text-ruby-400 border-ruby-500/20',
  slate:   'bg-slate-500/10 text-slate-400 border-slate-500/20',
};

/** Foreground accent (icons, section headers) in the given tone. */
export const TONE_TEXT: Record<RunStatusTone, string> = {
  emerald: 'text-emerald-400',
  cyan:    'text-cyan-400',
  violet:  'text-violet-400',
  amber:   'text-amber-400',
  ruby:    'text-ruby-400',
  slate:   'text-slate-500',
};

/** Left-border accent for list rows in the given tone. */
export const TONE_BORDER_L: Record<RunStatusTone, string> = {
  emerald: 'border-l-emerald-500/60',
  cyan:    'border-l-cyan-500/60',
  violet:  'border-l-violet-500/60',
  amber:   'border-l-amber-500/60',
  ruby:    'border-l-ruby-500/60',
  slate:   'border-l-slate-600/60',
};

/**
 * Mirror statuses that can never change again. Kept here (not in any one
 * component) because several surfaces key their stop-polling rule off it.
 */
export const TERMINAL_STATUSES = ['failed', 'cancelled', 'awaiting_mr', 'completed'];

/**
 * Buckets — the coarse triage grouping layered over the fine-grained
 * status vocabulary above (docs/REMOTE_EXECUTION_PLAN.md design §8's
 * taxonomy: PR ready / Failed / Parked / Needs credentials / Running /
 * Unreachable, plus `cancelled`, which isn't an outcome to chase and so
 * gets its own low-priority bucket rather than being crowbarred into
 * "Failed"). Statuses answer "what is this run doing?"; buckets answer
 * "does it want something from me, and how badly?" — which is what the
 * TopBar badge and FeatureDetail's action row key off.
 */
export type Bucket =
  | 'pr_ready'
  | 'failed'
  | 'parked'
  | 'needs_credentials'
  | 'running'
  | 'unreachable'
  | 'cancelled';

/** An unrecognised status is treated as in-motion, not failed: a run we
 *  can't name is more likely a status this build predates than a broken
 *  one, and "still running" is the safe thing to tell a human. */
export function bucketFor(status: string): Bucket {
  switch (status) {
    case 'awaiting_mr':
    case 'completed':
      return 'pr_ready';
    case 'failed':
    case 'interrupted':
      return 'failed';
    case 'parked':
    case 'over-budget':
      return 'parked';
    case 'needs-credentials':
      return 'needs_credentials';
    case 'unreachable':
      return 'unreachable';
    case 'cancelled':
      return 'cancelled';
    case 'pending':
    case 'running':
    default:
      return 'running';
  }
}
