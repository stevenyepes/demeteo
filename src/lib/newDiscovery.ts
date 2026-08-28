import type { Machine } from '../types';

/**
 * The two decisions the New-discovery modal makes before anything exists to
 * make them against, kept out of the component so both are reachable from a
 * test without a webview.
 *
 * `docs/PRD_DISCOVERY.md` §4.5 puts the host on the Discovery rather than
 * inheriting the project's, and §9.3 puts the vision warning where the model
 * is chosen rather than after the run. Both are answered here from data the
 * caller already holds.
 */

/** One entry of the machine select. */
export interface InterviewerMachineOption {
  id: string;
  label: string;
}

/** The desktop host. By policy it has no `machines` row, so it can never come
 *  out of the list and is always offered. */
export const LOCAL_MACHINE = 'local';

/**
 * Where the interview may run: this desktop, every configured machine, and —
 * if it is neither — whatever the project itself points at.
 *
 * That last case is why this is not `[local, ...machines]`. A project whose
 * host has since been deleted still names it, and a select with no option for
 * its own value silently shows the first one instead: the user would read a
 * machine they never chose and confirm it by pressing Start.
 */
export function interviewerMachineOptions(
  machines: readonly Machine[],
  projectMachineId: string,
): InterviewerMachineOption[] {
  const options: InterviewerMachineOption[] = [{ id: LOCAL_MACHINE, label: LOCAL_MACHINE }];
  for (const machine of machines) {
    if (machine.id === LOCAL_MACHINE) continue;
    options.push({ id: machine.id, label: machine.name || machine.id });
  }
  const project = projectMachineId.trim();
  if (project.length > 0 && !options.some((o) => o.id === project)) {
    options.push({ id: project, label: project });
  }
  return options;
}

/** What the §2.3 no-vision note names: the model that cannot read them, and
 *  the files it will be handed anyway. */
export interface NoVisionNote {
  model: string;
  filenames: string[];
}

/**
 * That note, or `null` when there is nothing honest to say.
 *
 * Soft by construction — it names what will happen and never withholds the
 * attachment, because the file is still worth attaching: the agent is told its
 * path either way, and only the inlining is lost.
 */
export function noVisionNote(args: {
  model: string;
  /** Whether the chosen model reads images. Probed where the caller holds the
   *  model list, name-matched where it does not. */
  readsImages: boolean;
  attachments: readonly { mime: string; name: string }[];
}): NoVisionNote | null {
  if (args.readsImages) return null;
  const filenames = args.attachments
    .filter((a) => a.mime.toLowerCase().startsWith('image/'))
    .map((a) => a.name);
  if (filenames.length === 0) return null;
  return { model: args.model.trim() || '(unset)', filenames };
}

/**
 * The longest a Discovery's name may be, mirroring
 * `domain::models::TITLE_MAX_CHARS`. Two copies because the backend is the
 * authority and the field has to say no before a round trip does — change one
 * and the `create` refusal is what the user meets instead of the counter.
 */
export const TITLE_MAX_CHARS = 80;

/** What the name field shows beneath itself. */
export interface NameFieldState {
  /** Characters left, negative once the cap is passed. */
  remaining: number;
  /** Whether the counter is worth drawing at all — a name nowhere near the
   *  cap does not need a number under it, and one that is always on reads as
   *  a limit being pressed rather than as a name being written. */
  showCounter: boolean;
  /** Whether the field is refusing what it holds. */
  overLimit: boolean;
}

/**
 * The name field's own readout of what it holds.
 *
 * The counter appears at the last quarter rather than from the first
 * character: `maxLength` stops typing at the cap silently, and a user whose
 * name has been cut off mid-word needs the number *then*, not before. It stays
 * reachable for a value the input never accepted a keystroke for — the seed
 * carried in from the hero card is set programmatically, so it can arrive past
 * the cap and has to be able to say so.
 */
export function nameFieldState(title: string): NameFieldState {
  const length = [...title.trim()].length;
  return {
    remaining: TITLE_MAX_CHARS - length,
    showCounter: length > TITLE_MAX_CHARS * 0.75,
    overLimit: length > TITLE_MAX_CHARS,
  };
}
