/**
 * Which workflow a new feature starts on (audit F10, UI redesign plan §5.2).
 *
 * The launch modal used to fall through to `workflows[0]` — whatever
 * `workflow_list` returned first — so the "default workflow" a project appeared
 * to have was an artefact of table order, and it moved whenever a workflow was
 * added, renamed or deleted. Everything below exists to replace that with a
 * stated rule: each degraded case has a decided answer, and none of the answers
 * depends on where a workflow sits in the list.
 *
 * `null` is one of those answers, not a failure. Where no rule *names* a
 * workflow the picker stays unselected and the user chooses, because launching
 * the wrong pipeline is not a cosmetic mistake — it is a different set of
 * agents doing different work in a worktree, and it is cheaper to ask than to
 * guess.
 */

/**
 * The starter pack's general-purpose pipeline, seeded on first launch from
 * `src-tauri/workflows/standard-feature-pipeline.json`. The id is stable across
 * versions and edits — `workflow_revert_to_default` republishes onto the same
 * row — which is what lets tier 3 below name one workflow instead of choosing
 * between seven starters by position. A project that deleted it falls through.
 */
export const STANDARD_STARTER_WORKFLOW_ID = 'wf-starter-standard';

export interface LaunchWorkflowInput {
  /** Every workflow the picker is currently offering. */
  workflows: ReadonlyArray<{ id: string }>;
  /** "Start a feature on *this* workflow" — an explicit intent the caller
   *  carried in from somewhere the user already pointed at a workflow. */
  requestedId?: string | null;
  /** `ProjectSettings.default_workflow_id`. Absent or `null` means the project
   *  has not chosen one, which is not the same as choosing none. */
  projectDefaultId?: string | null;
}

/**
 * Resolve the workflow a launch starts on, highest precedence first:
 *
 *  1. `requestedId` — the caller pointed at a workflow.
 *  2. `projectDefaultId` — the project chose one.
 *  3. {@link STANDARD_STARTER_WORKFLOW_ID} — the shipped general-purpose
 *     pipeline, present in every project that has not deleted it.
 *  4. The project's *only* workflow, when it has exactly one. Not "the first":
 *     with one candidate there is no choice to make and no order to depend on.
 *  5. `null` — several workflows, none of them named by a rule.
 *
 * An id that no longer resolves against `workflows` is skipped rather than
 * honoured: a stored default outlives the workflow it points at (deletes are
 * not cascaded), and a link or a deep link outlives both.
 */
export function resolveLaunchWorkflowId(input: LaunchWorkflowInput): string | null {
  const { workflows, requestedId, projectDefaultId } = input;
  const resolves = (id: string | null | undefined) => resolvesAgainst(id, workflows);

  if (resolves(requestedId)) return requestedId as string;
  if (resolves(projectDefaultId)) return projectDefaultId as string;
  if (resolves(STANDARD_STARTER_WORKFLOW_ID)) return STANDARD_STARTER_WORKFLOW_ID;
  if (workflows.length === 1) return workflows[0].id;

  return null;
}

/**
 * How the settings screen explains an unset default, exported rather than
 * written there because it *describes the ladder above*. Changing a tier and
 * leaving a hand-written sentence behind turns that screen into a confident
 * wrong answer about what the app will do — which is the exact shape of the
 * finding this module closes.
 */
export const UNSET_DEFAULT_WORKFLOW_HINT =
  'Which workflow a new feature starts on. Left unset, a launch falls back to the ' +
  'shipped standard pipeline — or asks, in a project that no longer has it. ' +
  'Overridable per feature either way.';

/** One spelling of "this id names a workflow that still exists", because the
 *  two answers below have to agree about it or the settings screen and the
 *  launch it configures disagree about what is stored. */
function resolvesAgainst(
  id: string | null | undefined,
  workflows: ReadonlyArray<{ id: string }>,
): id is string {
  return typeof id === 'string' && id.length > 0 && workflows.some((w) => w.id === id);
}

/**
 * What the *settings* screen shows for a stored `default_workflow_id`.
 *
 * It lives beside {@link resolveLaunchWorkflowId} because the two answer the
 * same question for different callers, and keeping them apart is how they start
 * disagreeing about what "resolves" means. They diverge on exactly one point,
 * deliberately: a launch **skips** an id that no longer resolves, while the
 * settings screen **hands it back**, because that screen is the only place a
 * dead choice can be corrected. A `<select>` whose value matches no option
 * renders blank — indistinguishable from "not set" and silent about why.
 *
 * `selected: ''` persists as `null`: the project has not chosen a workflow,
 * which is not the same as having chosen to have none.
 */
export interface DefaultWorkflowChoice {
  selected: string;
  /** The stored id no workflow answers to, or `null`. */
  dangling: string | null;
}

export function reconcileDefaultWorkflow(
  storedId: string | null | undefined,
  workflows: ReadonlyArray<{ id: string }>,
): DefaultWorkflowChoice {
  if (!storedId) return { selected: '', dangling: null };
  if (resolvesAgainst(storedId, workflows)) return { selected: storedId, dangling: null };
  return { selected: '', dangling: storedId };
}
