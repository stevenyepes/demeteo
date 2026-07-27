/**
 * `WorkflowBuilder` — design mode's owning screen (task P3.3, PRD §6.3).
 *
 * P3.1 gave the canvas editing gestures and P3.2 gave it a config panel, but
 * both are components: something has to own the *document* — the definition
 * being edited, its history, whether it is safe to leave, and whether it may be
 * saved. That is this file, and it is deliberately the only place with those
 * responsibilities:
 *
 * - **Validation surface.** `useWorkflowLint` round-trips the graph to the
 *   engine's own rule set (`workflow_lint`); findings become per-node badges on
 *   the canvas and a reason list here. Save is blocked by **errors only** —
 *   warnings are observations, not vetoes. The backend enforces the same rule
 *   at the write paths, so this is the friendly half of the guarantee, never
 *   the whole of it.
 * - **Dirty guard** (audit F38). Installed on the navigation context, so the
 *   Back arrow, `Escape`, `Cmd+W`, and the mouse back button are all covered by
 *   one prompt instead of three of them silently dropping a prompt template the
 *   author cannot retype.
 * - **Draft autosave.** Every 30s of unsaved work lands in `localStorage`, and
 *   the next open offers it back — the crash/reload case a navigation guard
 *   cannot see.
 * - **Undo/redo** over whole-definition snapshots, `⌘Z` / `⇧⌘Z`, suppressed
 *   while a text field has focus so it never fights the browser's own undo.
 *
 * Persistence is the owner's, via `onSave`: this screen produces a v2
 * definition and knows nothing about how workflows are stored. P3.6 wires the
 * route (and retires `WorkflowEditor`).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  History,
  OctagonAlert,
  Redo2,
  Save,
  Undo2,
} from 'lucide-react';

import { useErrorBus } from '../../lib/errorBus';
import { useNavigationGuard } from '../../hooks/useNavigationGuard';
import type { NavigationIntent } from '../../context/NavigationContext';
import { ConfigPanel } from './ConfigPanel';
import { WorkflowCanvas } from './WorkflowCanvas';
import { useGraphHistory } from './graphHistory';
import { describeFinding, lintSummary } from './lint';
import { useNodeTypes } from './nodeCatalog';
import { useWorkflowLint } from './useWorkflowLint';
import {
  clearDraft,
  DRAFT_AUTOSAVE_MS,
  loadDraft,
  saveDraft,
  type WorkflowDraft,
} from './workflowDraft';
import type { WorkflowDefinitionV2 } from './types';

export interface WorkflowSaveRequest {
  definition: WorkflowDefinitionV2;
  name: string;
  description: string;
}

export interface WorkflowBuilderProps {
  /** The workflow being edited; `null` for one that doesn't exist yet (the
   *  draft slot is keyed by this). */
  workflowId: string | null;
  /** Starting graph: a migrated v2 definition, a template, or a blank one. */
  definition: WorkflowDefinitionV2;
  name: string;
  description?: string;
  /** Latest saved version number, for the header chip. */
  version?: number;
  /** Persist the definition. Rejecting surfaces as an error toast and leaves
   *  the editor dirty; resolving marks it clean and clears the draft. */
  onSave: (request: WorkflowSaveRequest) => Promise<void>;
  /** Leave the builder. Called only once it is safe to do so. */
  onClose: () => void;
  className?: string;
}

/** What a blocked exit was trying to do, so it can be replayed. `'close'` is
 *  the builder's own Back arrow, which isn't a navigation intent. */
type PendingExit = { kind: 'intent'; intent: NavigationIntent } | { kind: 'close' };

/** Is the keystroke aimed at a text field? Then it belongs to that field's own
 *  undo stack, not the graph's. */
function isTextTarget(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el || !el.tagName) return false;
  const tag = el.tagName.toUpperCase();
  return (
    tag === 'INPUT' ||
    tag === 'TEXTAREA' ||
    tag === 'SELECT' ||
    el.isContentEditable === true
  );
}

function formatClock(ms: number): string {
  if (!ms) return 'earlier';
  try {
    return new Date(ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  } catch {
    return 'earlier';
  }
}

export function WorkflowBuilder({
  workflowId,
  definition,
  name: initialName,
  description: initialDescription = '',
  version,
  onSave,
  onClose,
  className = '',
}: WorkflowBuilderProps) {
  const { reportError } = useErrorBus();
  const { nodeTypes } = useNodeTypes();
  const history = useGraphHistory(definition);
  const { lint, checking } = useWorkflowLint(history.definition);

  const [name, setName] = useState(initialName);
  const [description, setDescription] = useState(initialDescription);
  /** The name/description as last persisted — the meta half of dirty. */
  const [savedMeta, setSavedMeta] = useState({
    name: initialName,
    description: initialDescription,
  });
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [pendingExit, setPendingExit] = useState<PendingExit | null>(null);
  const [draftOffer, setDraftOffer] = useState<WorkflowDraft | null>(null);

  const dirty =
    history.dirty || name !== savedMeta.name || description !== savedMeta.description;

  // ── Draft autosave / restore ────────────────────────────────────────────

  // Offer a stored draft when it says something the loaded definition doesn't.
  // Compared against the *loaded* graph so a draft that was saved and then
  // superseded isn't dangled in front of the author as if it were news.
  useEffect(() => {
    const stored = loadDraft(workflowId);
    if (!stored) return;
    if (JSON.stringify(stored.definition) === JSON.stringify(definition)) {
      clearDraft(workflowId);
      return;
    }
    setDraftOffer(stored);
  }, [workflowId, definition]);

  // Latest state in a ref so the autosave interval is installed once per dirty
  // transition rather than restarted by every keystroke — a restarted timer
  // would mean a fast typist never gets a save at all.
  const draftRef = useRef({ workflowId, name, description, definition: history.definition });
  draftRef.current = { workflowId, name, description, definition: history.definition };

  useEffect(() => {
    if (!dirty) return;
    const timer = setInterval(() => {
      const d = draftRef.current;
      saveDraft({
        workflowId: d.workflowId,
        name: d.name,
        description: d.description,
        definition: d.definition,
        savedAt: Date.now(),
      });
    }, DRAFT_AUTOSAVE_MS);
    return () => clearInterval(timer);
  }, [dirty]);

  const restoreDraft = useCallback(() => {
    if (!draftOffer) return;
    history.reset(draftOffer.definition, { dirty: true });
    setName(draftOffer.name || initialName);
    setDescription(draftOffer.description);
    setDraftOffer(null);
  }, [draftOffer, history, initialName]);

  const discardDraft = useCallback(() => {
    clearDraft(workflowId);
    setDraftOffer(null);
  }, [workflowId]);

  // ── Save ────────────────────────────────────────────────────────────────

  const blockingReasons = useMemo(
    () => lint.errors.map((f) => describeFinding(f, history.definition)),
    [lint.errors, history.definition],
  );

  const save = useCallback(async (): Promise<boolean> => {
    if (lint.hasErrors) {
      // Reachable via ⌘S even though the button is disabled — an author who
      // reaches for the shortcut deserves the reason, not silence.
      reportError(
        `Cannot save: ${blockingReasons.length} structural error${
          blockingReasons.length === 1 ? '' : 's'
        }.\n${blockingReasons.join('\n')}`,
        { kind: 'validation' },
      );
      return false;
    }
    if (!name.trim()) {
      reportError('Cannot save: the workflow needs a name.', { kind: 'validation' });
      return false;
    }
    setSaving(true);
    try {
      await onSave({ definition: history.definition, name: name.trim(), description });
      history.markSaved();
      setSavedMeta({ name: name.trim(), description });
      clearDraft(workflowId);
      return true;
    } catch (err) {
      reportError(err, { kind: 'validation' });
      return false;
    } finally {
      setSaving(false);
    }
  }, [
    lint.hasErrors,
    blockingReasons,
    name,
    description,
    history,
    onSave,
    workflowId,
    reportError,
  ]);

  // ── Exit guard (audit F38) ──────────────────────────────────────────────

  const { proceed } = useNavigationGuard(dirty, (intent) =>
    setPendingExit({ kind: 'intent', intent }),
  );

  const requestClose = useCallback(() => {
    if (dirty) setPendingExit({ kind: 'close' });
    else onClose();
  }, [dirty, onClose]);

  /** Perform the exit the guard held onto. */
  const performExit = useCallback(
    (exit: PendingExit) => {
      setPendingExit(null);
      if (exit.kind === 'close') onClose();
      else proceed(exit.intent);
    },
    [onClose, proceed],
  );

  const exitSaving = useCallback(async () => {
    const exit = pendingExit;
    if (!exit) return;
    if (await save()) performExit(exit);
    // A failed save keeps the prompt open: the work is still unsaved, and
    // dropping the user back into the editor with no explanation is how edits
    // get lost.
  }, [pendingExit, save, performExit]);

  const exitDiscarding = useCallback(() => {
    const exit = pendingExit;
    if (!exit) return;
    clearDraft(workflowId);
    performExit(exit);
  }, [pendingExit, workflowId, performExit]);

  // ── Keyboard: undo / redo / save ────────────────────────────────────────

  useEffect(() => {
    const onKeyDown = (evt: KeyboardEvent) => {
      if (!(evt.metaKey || evt.ctrlKey) || isTextTarget(evt.target)) return;
      const key = evt.key.toLowerCase();
      if (key === 'z') {
        evt.preventDefault();
        if (evt.shiftKey) history.redo();
        else history.undo();
      } else if (key === 'y') {
        // Windows/Linux redo.
        evt.preventDefault();
        history.redo();
      } else if (key === 's') {
        evt.preventDefault();
        void save();
      }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [history, save]);

  // ── Render ──────────────────────────────────────────────────────────────

  const summary = lintSummary(lint);
  const selectedExists = selectedNodeId
    ? history.definition.nodes.some((n) => n.id === selectedNodeId)
    : false;

  return (
    <div
      className={`relative flex h-full min-h-0 flex-col bg-[#0b0d12] ${className}`}
      data-testid="workflow-builder"
    >
      <header className="flex items-center gap-3 border-b border-white/5 px-4 py-3">
        <button
          type="button"
          onClick={requestClose}
          className="rounded-lg border border-slate-700/60 p-1.5 text-slate-300 transition-colors hover:border-slate-600 hover:text-white"
          aria-label="Back to workflows"
        >
          <ArrowLeft className="h-4 w-4" />
        </button>

        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Workflow name"
          aria-label="Workflow name"
          className="min-w-0 flex-1 rounded-lg border border-transparent bg-transparent px-2 py-1 text-base font-medium text-slate-100 outline-none transition-colors hover:border-slate-700/60 focus:border-cyan-500/50"
        />

        {typeof version === 'number' && version > 0 && (
          <span className="flex items-center gap-1 rounded border border-slate-700/60 px-1.5 py-0.5 text-[10px] font-mono text-slate-400">
            <History className="h-3 w-3" />v{version}
          </span>
        )}

        {dirty && (
          <span
            className="text-[11px] font-medium text-amber-300"
            data-testid="dirty-indicator"
          >
            Unsaved
          </span>
        )}

        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={history.undo}
            disabled={!history.canUndo}
            title="Undo (⌘Z)"
            aria-label="Undo"
            className="rounded-lg border border-slate-700/60 p-1.5 text-slate-300 transition-colors hover:border-slate-600 hover:text-white disabled:opacity-40"
          >
            <Undo2 className="h-4 w-4" />
          </button>
          <button
            type="button"
            onClick={history.redo}
            disabled={!history.canRedo}
            title="Redo (⇧⌘Z)"
            aria-label="Redo"
            className="rounded-lg border border-slate-700/60 p-1.5 text-slate-300 transition-colors hover:border-slate-600 hover:text-white disabled:opacity-40"
          >
            <Redo2 className="h-4 w-4" />
          </button>
        </div>

        <span
          data-testid="lint-status"
          title={
            lint.findings.length > 0
              ? lint.findings.map((f) => describeFinding(f, history.definition)).join('\n')
              : 'No structural issues'
          }
          className={[
            'flex items-center gap-1.5 rounded-lg border px-2 py-1 text-[11px] font-medium',
            lint.hasErrors
              ? 'border-rose-500/30 bg-rose-500/10 text-rose-300'
              : lint.warnings.length > 0
                ? 'border-amber-500/30 bg-amber-500/10 text-amber-300'
                : 'border-emerald-500/30 bg-emerald-500/10 text-emerald-300',
          ].join(' ')}
        >
          {lint.hasErrors ? (
            <OctagonAlert className="h-3.5 w-3.5" />
          ) : lint.warnings.length > 0 ? (
            <AlertTriangle className="h-3.5 w-3.5" />
          ) : (
            <Check className="h-3.5 w-3.5" />
          )}
          {summary ?? (checking ? 'Checking…' : 'Valid')}
        </span>

        <button
          type="button"
          onClick={() => void save()}
          disabled={saving || lint.hasErrors}
          title={lint.hasErrors ? blockingReasons.join('\n') : 'Save a new version (⌘S)'}
          className="flex items-center gap-1.5 rounded-lg border border-cyan-500/40 bg-cyan-500/10 px-3 py-1.5 text-xs font-medium text-cyan-200 transition-colors hover:border-cyan-400/60 hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Save className="h-3.5 w-3.5" />
          {saving ? 'Saving…' : 'Save'}
        </button>
      </header>

      {draftOffer && (
        <div
          className="flex items-center gap-3 border-b border-amber-500/20 bg-amber-500/5 px-4 py-2 text-xs text-amber-200"
          data-testid="draft-offer"
        >
          <History className="h-3.5 w-3.5 shrink-0" />
          <span className="flex-1">
            An unsaved draft from {formatClock(draftOffer.savedAt)} was recovered.
          </span>
          <button
            type="button"
            onClick={restoreDraft}
            className="rounded border border-amber-500/40 px-2 py-0.5 font-medium transition-colors hover:bg-amber-500/10"
          >
            Restore
          </button>
          <button
            type="button"
            onClick={discardDraft}
            className="rounded border border-slate-600/60 px-2 py-0.5 font-medium text-slate-300 transition-colors hover:bg-slate-700/30"
          >
            Discard
          </button>
        </div>
      )}

      {lint.hasErrors && (
        <ul
          className="border-b border-rose-500/20 bg-rose-500/5 px-4 py-2 text-xs text-rose-200"
          data-testid="lint-errors"
        >
          {blockingReasons.map((reason) => (
            <li key={reason} className="flex gap-2">
              <OctagonAlert className="mt-0.5 h-3 w-3 shrink-0" />
              <span>{reason}</span>
            </li>
          ))}
        </ul>
      )}

      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1">
          <WorkflowCanvas
            definition={history.definition}
            mode="design"
            nodeTypes={nodeTypes}
            lint={lint}
            selectedNodeId={selectedNodeId}
            onDefinitionChange={history.commit}
            onConnectRejected={(message) => reportError(message, { kind: 'validation' })}
            onNodeActivate={(nodeId) =>
              setSelectedNodeId((prev) => (prev === nodeId ? null : nodeId))
            }
          />
        </div>
        {selectedNodeId && selectedExists && (
          <ConfigPanel
            definition={history.definition}
            nodeId={selectedNodeId}
            nodeTypes={nodeTypes}
            onChange={history.commit}
            onClose={() => setSelectedNodeId(null)}
          />
        )}
      </div>

      {pendingExit && (
        <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
          <div
            role="dialog"
            aria-label="Unsaved changes"
            data-testid="dirty-guard"
            className="w-[420px] rounded-xl border border-slate-700/60 bg-[#0d0f14] p-5 shadow-2xl"
          >
            <h2 className="text-sm font-semibold text-slate-100">Unsaved changes</h2>
            <p className="mt-2 text-xs leading-relaxed text-slate-400">
              This workflow has edits that were never saved as a version. Leaving now
              discards them.
            </p>
            <div className="mt-4 flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setPendingExit(null)}
                className="rounded-lg border border-slate-700/60 px-3 py-1.5 text-xs font-medium text-slate-300 transition-colors hover:border-slate-600 hover:text-white"
              >
                Keep editing
              </button>
              <button
                type="button"
                onClick={exitDiscarding}
                className="rounded-lg border border-rose-500/40 bg-rose-500/10 px-3 py-1.5 text-xs font-medium text-rose-200 transition-colors hover:border-rose-400/60"
              >
                Discard
              </button>
              <button
                type="button"
                onClick={() => void exitSaving()}
                disabled={saving || lint.hasErrors}
                title={lint.hasErrors ? blockingReasons.join('\n') : undefined}
                className="rounded-lg border border-cyan-500/40 bg-cyan-500/10 px-3 py-1.5 text-xs font-medium text-cyan-200 transition-colors hover:border-cyan-400/60 disabled:cursor-not-allowed disabled:opacity-40"
              >
                Save and leave
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
