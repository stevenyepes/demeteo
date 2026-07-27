/**
 * Local draft autosave for the builder (task P3.3, audit F38's other half).
 *
 * The dirty guard stops a *navigation* from dropping unsaved work; it can do
 * nothing about a crash, a reload, or the webview being torn down. So the
 * builder also parks its in-progress definition in `localStorage` every 30s and
 * offers to restore it on the next open.
 *
 * Deliberately **not** the backend: a draft is not a `WorkflowVersion`. Every
 * save mints an immutable version row (PRD §6.3) and half-finished graphs must
 * not pollute that history — nor should they be pinnable by a run. Losing a
 * draft is survivable; a bogus version row is not.
 */
import type { WorkflowDefinitionV2 } from './types';

const STORAGE_PREFIX = 'demeteo.workflow.draft.';

/** Autosave cadence (PRD §6.3: "local draft autosave every 30s"). */
export const DRAFT_AUTOSAVE_MS = 30_000;

/** Bump when the stored shape changes so old drafts are ignored, not
 *  half-read. */
const DRAFT_FORMAT = 1;

export interface WorkflowDraft {
  format: number;
  /** The workflow being edited; `null` for a not-yet-created one. */
  workflowId: string | null;
  name: string;
  description: string;
  definition: WorkflowDefinitionV2;
  /** Epoch ms — shown in the restore prompt so the author can judge it. */
  savedAt: number;
}

/** One draft slot per workflow, plus a `new` slot for unsaved creations. */
export function draftKey(workflowId: string | null): string {
  return `${STORAGE_PREFIX}${workflowId ?? 'new'}`;
}

/** Persist a draft. Never throws — a full/blocked localStorage must not take
 *  down the editor the draft exists to protect. */
export function saveDraft(draft: Omit<WorkflowDraft, 'format'>): void {
  if (typeof localStorage === 'undefined') return;
  try {
    const payload: WorkflowDraft = { ...draft, format: DRAFT_FORMAT };
    localStorage.setItem(draftKey(draft.workflowId), JSON.stringify(payload));
  } catch {
    // Ignored on purpose: see above.
  }
}

/** Read the stored draft, or `null` when there is none / it is unreadable. */
export function loadDraft(workflowId: string | null): WorkflowDraft | null {
  if (typeof localStorage === 'undefined') return null;
  try {
    const raw = localStorage.getItem(draftKey(workflowId));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<WorkflowDraft>;
    if (parsed?.format !== DRAFT_FORMAT) return null;
    const def = parsed.definition;
    // Minimum viable graph: a definition we can actually put on the canvas.
    if (!def || !Array.isArray(def.nodes) || !Array.isArray(def.edges)) return null;
    return {
      format: DRAFT_FORMAT,
      workflowId: parsed.workflowId ?? null,
      name: typeof parsed.name === 'string' ? parsed.name : '',
      description: typeof parsed.description === 'string' ? parsed.description : '',
      definition: def,
      savedAt: typeof parsed.savedAt === 'number' ? parsed.savedAt : 0,
    };
  } catch {
    return null;
  }
}

export function clearDraft(workflowId: string | null): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.removeItem(draftKey(workflowId));
  } catch {
    // Ignored: a draft we can't clear is stale at worst, and `loadDraft`'s
    // caller compares it against the saved definition before offering it.
  }
}
