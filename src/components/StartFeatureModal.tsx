import React, { useState, useEffect, useMemo, useRef } from 'react';
import { X, Sparkles, GitBranch, AlertTriangle, ChevronDown, ChevronUp, Cpu } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';

interface Repository {
  id: string;
  repo_path: string;
}

interface WorkflowSummary {
  id: string;
  name: string;
  description: string;
  version: number;
}

interface StartFeatureModalProps {
  isOpen: boolean;
  projectId: string;
  /** Available workflows — the user picks one. */
  workflows: WorkflowSummary[];
  /** Repos attached to the project. Used to infer chips and detect conflicts. */
  repositories: Repository[];
  /** Display name for the project (shown in the header). */
  projectName?: string;
  /** Pre-select a specific workflow id (e.g. the one the user clicked). */
  defaultWorkflowId?: string | null;
  onClose: () => void;
  /**
   * Called with the resolved launch parameters when the user clicks
   * "Launch feature". The parent is responsible for invoking
   * `start_feature` (Tauri command) and surfacing errors.
   *
   * `commitArtifacts` is the per-feature override for
   * `ProjectSettings.commit_artifacts`. `undefined` → inherit the
   * project default. See migration V12.
   */
  onLaunch: (params: {
    workflowId: string;
    title: string;
    description: string;
    agentKind?: string;
    model?: string;
    targetRepos: string[];
    commitArtifacts?: boolean;
    /** Per-run override of the loop iteration budget (migration V13). */
    loopIterations?: number;
    /** Per-step agent/model overrides chosen at launch (migration V13). */
    stepOverrides?: { step_id: string; agent_kind?: string | null; model?: string | null }[];
  }) => void;
}

const AGENT_KINDS = ['opencode', 'hermes', 'claude-code', 'antigravity'];

interface StepRow {
  id: string;
  title: string;
  kind: string;
}

/**
 * The slim "Start a feature" modal (Q22).
 *
 * - Always-visible: title + description textarea + workflow picker.
 * - Inferred chips: as the user types, the modal scans the description
 *   for repo-name keywords and shows the matching repos as chips. A
 *   repo that's already used by an active feature gets a `Conflict`
 *   badge.
 * - "Customize…" expansion: opens the advanced section (agent kind,
 *   model override, target repos override, and a per-feature override
 *   for whether agent reports are committed to the PR). Collapsed by
 *   default per Q22's "slim" framing.
 *
 * No LLM is invoked from this modal — inference is local keyword
 * matching per Q25.
 */
const StartFeatureModal: React.FC<StartFeatureModalProps> = ({
  isOpen,
  projectId,
  workflows,
  repositories,
  projectName,
  defaultWorkflowId,
  onClose,
  onLaunch,
}) => {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [workflowId, setWorkflowId] = useState<string>('');
  const [agentKind, setAgentKind] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [conflicts, setConflicts] = useState<Set<string>>(new Set());
  // Steps of the selected workflow + per-step agent/model overrides.
  // A blank entry means "inherit" the workflow/project default for that step.
  const [steps, setSteps] = useState<StepRow[]>([]);
  const [stepOverrides, setStepOverrides] = useState<Record<string, { agent_kind: string; model: string }>>({});
  // Per-run loop budget. Empty string = inherit project/engine default.
  const [loopIterations, setLoopIterations] = useState<string>('');
  // Per-feature override for the project's `commit_artifacts` setting.
  // `'inherit'` is the default — pass `undefined` to `start_feature`
  // so the project default applies. `'yes'` / `'no'` become a
  // concrete `true` / `false` on the Feature row. See migration V12.
  const [commitArtifacts, setCommitArtifacts] = useState<'inherit' | 'yes' | 'no'>('inherit');
  const titleRef = useRef<HTMLInputElement>(null);

  // Initialize workflow picker to the requested default (or the first
  // workflow if none specified) when the modal opens.
  useEffect(() => {
    if (isOpen) {
      if (defaultWorkflowId && workflows.some((w) => w.id === defaultWorkflowId)) {
        setWorkflowId(defaultWorkflowId);
      } else if (workflows.length > 0 && !workflowId) {
        setWorkflowId(workflows[0].id);
      }
      setTimeout(() => titleRef.current?.focus(), 0);
    } else {
      // reset on close so the next open is clean
      setTitle('');
      setDescription('');
      setAgentKind('');
      setModel('');
      setShowAdvanced(false);
      setCommitArtifacts('inherit');
      setSteps([]);
      setStepOverrides({});
      setLoopIterations('');
    }
  }, [isOpen, workflows, defaultWorkflowId, workflowId]);

  // Load the selected workflow's steps so the user can override the agent /
  // model per step. Gate steps don't run an agent, so they're filtered out.
  useEffect(() => {
    if (!isOpen || !workflowId) {
      setSteps([]);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const w: any = await invoke('workflow_get', { workflowId });
        if (cancelled) return;
        const rows: StepRow[] = (w.steps || [])
          .filter((s: any) => s.kind !== 'gate')
          .map((s: any) => ({ id: s.id, title: s.title, kind: s.kind }));
        setSteps(rows);
      } catch (e) {
        console.warn('failed to load workflow steps for per-step overrides:', e);
        setSteps([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, workflowId]);

  // Q25: keyword-based repo inference. No LLM, no network.
  const inferredRepos = useMemo(() => {
    if (!description.trim()) return [] as Repository[];
    const haystack = description.toLowerCase();
    return repositories.filter((r) => {
      const path = r.repo_path.toLowerCase();
      // The last path segment ("owner/repo") is the strongest signal.
      const segments = path.split(/[/.]/).filter(Boolean);
      return segments.some((seg) => seg.length >= 2 && haystack.includes(seg));
    });
  }, [description, repositories]);

  // Q26 / Q11 — detect repos already used by another active feature
  // (so we can warn the user before they kick off a parallel run).
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    (async () => {
      try {
        const active: any[] = await invoke('fetch_active_features', { projectId });
        if (cancelled) return;
        const usedRepos = new Set<string>();
        for (const f of active) {
          const fRepos: any[] = await invoke('get_repositories_for_project', { projectId: f.project_id });
          for (const fr of fRepos) {
            if (f.id !== /* self */ undefined) usedRepos.add(fr.id);
          }
        }
        if (cancelled) return;
        const inUse = new Set<string>();
        for (const r of repositories) {
          if (usedRepos.has(r.id)) inUse.add(r.id);
        }
        setConflicts(inUse);
      } catch (e) {
        // Soft fail — modal still works, we just skip the conflict warning.
        console.warn('conflict detection failed:', e);
        setConflicts(new Set());
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, projectId, repositories]);

  if (!isOpen) return null;

  const canLaunch = title.trim().length > 0 && description.trim().length > 0 && workflowId !== '';

  const launch = () => {
    if (!canLaunch) return;
    const targetRepos = inferredRepos.length > 0 ? inferredRepos.map((r) => r.id) : repositories.map((r) => r.id);
    const commitArtifactsArg =
      commitArtifacts === 'inherit'
        ? undefined
        : commitArtifacts === 'yes';
    // Only emit rows where the user actually set an agent or model.
    const overrides = Object.entries(stepOverrides)
      .map(([step_id, v]) => ({
        step_id,
        agent_kind: v.agent_kind.trim() || null,
        model: v.model.trim() || null,
      }))
      .filter((o) => o.agent_kind || o.model);
    const loopArg = loopIterations.trim() ? parseInt(loopIterations, 10) : undefined;
    onLaunch({
      workflowId,
      title: title.trim(),
      description: description.trim(),
      agentKind: agentKind.trim() || undefined,
      model: model.trim() || undefined,
      targetRepos,
      commitArtifacts: commitArtifactsArg,
      loopIterations: Number.isFinite(loopArg as number) ? loopArg : undefined,
      stepOverrides: overrides.length > 0 ? overrides : undefined,
    });
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === 'Escape') {
      onClose();
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      launch();
    }
  };

  return (
    <div
      className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-[60] p-4 select-none"
      onKeyDown={onKey}
    >
      <div className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-2xl shadow-2xl overflow-hidden">
        <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508]">
          <div className="flex items-center gap-2">
            <Sparkles className="w-4 h-4 text-cyan-400" />
            <h3 className="text-sm font-semibold text-white">
              Start a feature {projectName ? <span className="text-slate-400">· {projectName}</span> : null}
            </h3>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-slate-500 hover:text-white transition-colors"
            aria-label="Close"
          >
            <X size={16} />
          </button>
        </div>

        <div className="p-6 space-y-4">
          {/* Title */}
          <div>
            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
              Title
            </label>
            <input
              ref={titleRef}
              type="text"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="e.g. Add OAuth2 login flow"
              className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
            />
          </div>

          {/* Description */}
          <div>
            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
              Describe the feature
            </label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={5}
              placeholder="What does this feature do? Who uses it? Any constraints, edge cases, or non-goals? Reference repo names — the modal will auto-detect them."
              className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 resize-y"
            />
          </div>

          {/* Workflow picker (always visible per Q22) */}
          <div>
            <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
              Workflow
            </label>
            <select
              value={workflowId}
              onChange={(e) => setWorkflowId(e.target.value)}
              className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
            >
              {workflows.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.name} (v{w.version})
                </option>
              ))}
            </select>
          </div>

          {/* Inferred repo chips (Q25 — local keyword matching, no LLM) */}
          {repositories.length > 0 && (
            <div>
              <div className="flex items-center gap-2 mb-2">
                <GitBranch className="w-3.5 h-3.5 text-violet-400" />
                <span className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                  Target repositories
                </span>
                <span className="text-[10px] font-mono text-slate-500">
                  (auto-detected from description; edit in Customize)
                </span>
              </div>
              <div className="flex flex-wrap gap-2">
                {inferredRepos.length === 0 ? (
                  <span className="text-xs text-slate-500 italic">
                    No repos mentioned — will run against all project repos.
                  </span>
                ) : (
                  inferredRepos.map((r) => {
                    const inUse = conflicts.has(r.id);
                    return (
                      <span
                        key={r.id}
                        className={`inline-flex items-center gap-1.5 text-xs font-mono px-2.5 py-1 rounded-md border ${
                          inUse
                            ? 'border-amber-500/40 bg-amber-500/10 text-amber-200'
                            : 'border-cyan-500/30 bg-cyan-500/10 text-cyan-200'
                        }`}
                        title={r.repo_path}
                      >
                        <GitBranch className="w-3 h-3" />
                        {r.repo_path}
                        {inUse && (
                          <span className="flex items-center gap-1 ml-1 text-amber-300">
                            <AlertTriangle className="w-3 h-3" />
                            conflict
                          </span>
                        )}
                      </span>
                    );
                  })
                )}
              </div>
            </div>
          )}

          {/* Customize (Q22: expand to full form) */}
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="flex items-center gap-1.5 text-xs text-cyan-300 hover:text-cyan-200 transition-colors"
          >
            {showAdvanced ? <ChevronUp className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
            {showAdvanced ? 'Hide' : 'Customize…'}
          </button>

          {showAdvanced && (
            <div className="space-y-3 pl-3 border-l border-white/5">
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                    Default agent — all steps
                  </label>
                  <input
                    type="text"
                    value={agentKind}
                    onChange={(e) => setAgentKind(e.target.value)}
                    placeholder="blank = project default"
                    className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
                  />
                </div>
                <div>
                  <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                    Default model — all steps
                  </label>
                  <input
                    type="text"
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder="claude-opus-4-8, …"
                    className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
                  />
                </div>
              </div>

              {/* Per-step agent/model overrides. Blank row = inherit the
                  default above → the workflow step → the project default. */}
              {steps.length > 0 && (
                <div>
                  <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                    Per-step overrides (optional)
                  </label>
                  <div className="space-y-1.5">
                    {steps.map((s, i) => {
                      const ov = stepOverrides[s.id] || { agent_kind: '', model: '' };
                      const setOv = (patch: Partial<{ agent_kind: string; model: string }>) =>
                        setStepOverrides((prev) => {
                          const cur = prev[s.id] || { agent_kind: '', model: '' };
                          return { ...prev, [s.id]: { ...cur, ...patch } };
                        });
                      return (
                        <div key={s.id} className="flex items-center gap-2">
                          <span
                            className="text-[11px] text-slate-400 font-mono w-40 shrink-0 truncate"
                            title={s.title}
                          >
                            {i + 1}. {s.title}
                          </span>
                          <select
                            value={ov.agent_kind}
                            onChange={(e) => setOv({ agent_kind: e.target.value })}
                            className="flex-1 min-w-0 bg-[#050508] border border-white/10 rounded-lg px-2 py-1.5 text-[11px] text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
                          >
                            <option value="">inherit</option>
                            {AGENT_KINDS.map((ak) => (
                              <option key={ak} value={ak}>
                                {ak}
                              </option>
                            ))}
                          </select>
                          <input
                            type="text"
                            value={ov.model}
                            onChange={(e) => setOv({ model: e.target.value })}
                            placeholder="model (inherit)"
                            className="flex-1 min-w-0 bg-[#050508] border border-white/10 rounded-lg px-2 py-1.5 text-[11px] text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                          />
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

              <div>
                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                  Loop iterations (optional)
                </label>
                <input
                  type="number"
                  min={1}
                  max={10}
                  value={loopIterations}
                  onChange={(e) => setLoopIterations(e.target.value)}
                  placeholder="blank = project default (3)"
                  className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                />
                <p className="text-[10px] font-mono text-slate-500 mt-1.5 leading-relaxed">
                  Max times a validation step loops back to implementation before giving up.
                </p>
              </div>

              <div>
                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                  Commit artifacts to PR
                </label>
                <div className="grid grid-cols-3 gap-1.5">
                  {([
                    { key: 'inherit', label: 'Project default', desc: 'Inherit' },
                    { key: 'yes', label: 'Yes', desc: 'Ship reports in the PR' },
                    { key: 'no', label: 'No', desc: 'Keep in demeteo only' },
                  ] as const).map((opt) => (
                    <button
                      key={opt.key}
                      type="button"
                      onClick={() => setCommitArtifacts(opt.key)}
                      className={`px-2.5 py-2 rounded-lg text-[11px] font-semibold uppercase tracking-wider border transition-colors ${
                        commitArtifacts === opt.key
                          ? 'bg-cyan-500/15 border-cyan-500/40 text-cyan-200'
                          : 'bg-[#050508] border-white/10 text-slate-400 hover:border-white/20'
                      }`}
                      title={opt.desc}
                    >
                      {opt.label}
                    </button>
                  ))}
                </div>
                <p className="text-[10px] font-mono text-slate-500 mt-1.5 leading-relaxed">
                  Each step produces a report (<code>research-report.md</code>, <code>critic-review.md</code>, …). The project's default is configured in project settings.
                </p>
              </div>
              <div className="flex items-start gap-2 text-[11px] text-slate-500">
                <Cpu className="w-3.5 h-3.5 mt-0.5 text-slate-600" />
                <span>
                  Per-step cost is backfilled from the active agent's <code className="text-slate-400">Usage</code> event,
                  with the pricing table as a fallback.
                </span>
              </div>
            </div>
          )}
        </div>

        <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex justify-between items-center">
          <span className="text-[10px] text-slate-500 font-mono">
            {canLaunch ? '⌘/Ctrl + Enter to launch' : 'Fill in title, description, and workflow to launch'}
          </span>
          <div className="flex gap-3">
            <button
              type="button"
              onClick={onClose}
              className="px-4 py-2 rounded-lg text-xs font-medium text-slate-400 hover:text-white transition-colors"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={launch}
              disabled={!canLaunch}
              className={`px-5 py-2 rounded-lg text-xs font-bold transition-all ${
                canLaunch
                  ? 'bg-cyan-500 text-slate-950 hover:bg-cyan-400'
                  : 'bg-white/5 text-slate-600 cursor-not-allowed'
              }`}
            >
              Launch feature
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default StartFeatureModal;
