import React, { useState, useEffect, useMemo, useRef } from 'react';
import { X, Sparkles, GitBranch, AlertTriangle, ChevronDown, ChevronUp, Cpu, EyeOff, Server, MoonStar } from 'lucide-react';
import type { EffortLevel, FeatureOrigin, Machine, Repository, WorkflowSummary } from '../types';
import { AttachmentDropzone, type LaunchStageEntry } from './AttachmentDropzone';
import { extractClipboardImageFiles, recoverClipboardImageFile, stageBrowserFilesForLaunch } from '../lib/attachments';
import { formatError } from '../lib/errors';
import { modelSupportsImagesByName } from '../lib/modelImageSupport';
import { getAgentModels } from '../lib/agentModels';
import { effortLevelsFor, useAgentCatalog } from '../lib/agentCatalog';
import { reconcileEffort } from '../lib/effortLevels';
import { HarnessModelPicker, type ModelOption } from './ui/HarnessModelPicker';
import {
  getAgentConfigs,
  listMachines,
  type AgentConfigView,
} from '../lib/machines';
import { getProposedStrategy, getRepositoriesForProject } from '../lib/project';
import { fetchActiveFeatures } from '../lib/features';
import { getWorkflow, listWorkflows, workflowVersionGraph } from '../lib/workflows';
import { resolveLaunchWorkflowId } from '../lib/workflowDefault';
import { MiniGraph } from './canvas/MiniGraph';
import { OriginPicker } from './StartFeatureModal/OriginPicker';
import { runOriginArgs, type OriginSelection } from '../lib/runOrigin';
import type { WorkflowDefinitionV2 } from './canvas/types';

interface StartFeatureModalProps {
  isOpen: boolean;
  projectId: string;
  /** Repos attached to the project. Used to infer chips and detect conflicts. */
  repositories: Repository[];
  /** Display name for the project (shown in the header). */
  projectName?: string;
  /** The project's `compute_type` ('local' | 'remote'). When 'remote',
   * a machine-less launch is attached-remote: the desktop app
   * orchestrates over SSH against `remoteHost` — a project-level fact
   * the "Where to run" section states rather than a per-run choice. */
  computeType?: string;
  /** The project's `remote_host` machine id (attached-remote only). */
  remoteHost?: string | null;
  /** Pre-select a specific workflow id (e.g. the one the user clicked). */
  defaultWorkflowId?: string | null;
  /**
   * Prefill from the inline composer on ProjectHome (Alternative A).
   * Applied once on open, and only into still-empty fields so a user
   * mid-edit is never clobbered. The modal owns the launch from here.
   */
  seedTitle?: string;
  seedAttachments?: LaunchStageEntry[];
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
    /** Feature-wide reasoning effort chosen at launch. Unset = inherit. */
    effort?: EffortLevel;
    targetRepos: string[];
    commitArtifacts?: boolean;
    /** Per-run override of the loop iteration budget (migration V13). */
    loopIterations?: number;
    /** Per-run override of the per-turn dollar budget, `--max-budget-usd`
     *  (migration V30). Unset = inherit project/engine default ($20). */
    maxBudgetUsd?: number;
    /** Per-step agent/model/effort overrides chosen at launch (migration V13). */
    stepOverrides?: { step_id: string; agent_kind?: string | null; model?: string | null; effort?: EffortLevel | null }[];
    /**
     * Staged file attachments, keyed by sha256 (see
     * `AttachmentDropzone.tsx`). The modal cannot persist these
     * itself — it has no `feature_id` until `start_feature` returns —
     * so the parent is responsible for committing them via
     * `feature_add_attachment(featureId, …)` after launch.
     */
    attachments?: LaunchStageEntry[];
    /**
     * Run this feature on a remote `demeteo-runner` instead of locally
     * (docs/REMOTE_EXECUTION.md M6.1). `undefined`/`'local'` means
     * "run here" — the existing `start_feature` path. Any other value
     * is a machine id the parent resolves via `remote_submit_run`.
     */
    machineId?: string;
    /** R6/R7: auto-approve safe gates, park dangerous ones. Only
     * meaningful when `machineId` is set. */
    unattended?: boolean;
    /** M5.2 hard caps — `undefined` means no cap on that dimension. */
    maxCostUsd?: number;
    maxWallClockMins?: number;
    /** Where the run's branch is cut from (migration V41). Both of these are
     *  absent for a run that started where every run started before the
     *  origin picker — see `src/lib/runOrigin.ts`. */
    origin?: FeatureOrigin;
    diffBaseBranch?: string;
  }) => void;
}

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
  repositories,
  projectName,
  computeType,
  remoteHost,
  defaultWorkflowId,
  seedTitle,
  seedAttachments,
  onClose,
  onLaunch,
}) => {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  // Workflow templates the user picks from. Fetched by the modal itself
  // on open (like `machines` below) so every entry point — command
  // palette, keyboard shortcut, Workflows page — sees a populated list;
  // relying on a caller to pre-load it left the picker empty from most
  // of them.
  const [workflows, setWorkflows] = useState<WorkflowSummary[]>([]);
  const [workflowId, setWorkflowId] = useState<string>('');
  // Both loads answer the same question — "what should the picker start on?" —
  // and both have a legitimate empty answer, so an empty list and an unset
  // project default are indistinguishable from "still loading" by value alone.
  // These flags are what let the seed wait for a real answer instead of
  // committing to a fallback the first render would have produced.
  const [workflowsLoaded, setWorkflowsLoaded] = useState(false);
  const [projectDefaultWorkflowId, setProjectDefaultWorkflowId] = useState<string | null>(null);
  const [projectDefaultLoaded, setProjectDefaultLoaded] = useState(false);
  const [agentKind, setAgentKind] = useState<string>('');
  const [model, setModel] = useState<string>('');
  const [effort, setEffort] = useState<EffortLevel | ''>('');
  // Agents actually configured/available on the target machine (or the
  // project's local config when running here). Drives the agent pickers
  // below instead of a hardcoded list, so the modal never offers an
  // agent that isn't installed on the chosen machine — the same
  // `get_agent_configs` probe the Strategy settings tab uses.
  const [agentConfigs, setAgentConfigs] = useState<AgentConfigView[]>([]);
  const { agents: agentCatalog } = useAgentCatalog();
  // Model lists per agent kind, probed lazily from the target machine via
  // `getAgentModels` — the same `get_agent_models` command the Strategy
  // settings tab and Project home use. Keyed by agent kind (not machine)
  // because the map is cleared whenever the target machine changes; the
  // underlying `getAgentModels` cache is keyed by `(machine, agent)` and
  // dedupes concurrent probes. A key present in `modelsLoading` but absent
  // from `modelsByAgent` means an in-flight probe.
  const [modelsByAgent, setModelsByAgent] = useState<Record<string, ModelOption[]>>({});
  const [modelsLoading, setModelsLoading] = useState<Record<string, boolean>>({});
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [conflicts, setConflicts] = useState<Set<string>>(new Set());
  // Explicit target-repo selection. `null` means "follow auto-detect"
  // (repos inferred from the description, or all project repos when none
  // are mentioned); a non-null array is the user's explicit choice made
  // in Customize. Kept as an override so untouched launches behave
  // exactly as before.
  const [repoOverride, setRepoOverride] = useState<string[] | null>(null);
  const [originSelection, setOriginSelection] = useState<OriginSelection>({
    base: null,
    diffBase: null,
  });
  // Steps of the selected workflow + per-step agent/model overrides.
  // A blank entry means "inherit" the workflow/project default for that step.
  const [steps, setSteps] = useState<StepRow[]>([]);
  // The pinned version's graph, for the shape preview (P3.6). `null` while
  // loading or when the read failed — the preview is informational.
  const [graph, setGraph] = useState<WorkflowDefinitionV2 | null>(null);
  const [stepOverrides, setStepOverrides] = useState<Record<string, { agent_kind: string; model: string; effort: EffortLevel | '' }>>({});
  // Per-run loop budget. Empty string = inherit project/engine default.
  const [loopIterations, setLoopIterations] = useState<string>('');
  // Per-run, per-turn dollar budget (--max-budget-usd). Empty = inherit
  // project/engine default. Applies on all paths, not just detached.
  const [maxBudgetUsd, setMaxBudgetUsd] = useState<string>('');
  // Per-feature override for the project's `commit_artifacts` setting.
  // `'inherit'` is the default — pass `undefined` to `start_feature`
  // so the project default applies. `'yes'` / `'no'` become a
  // concrete `true` / `false` on the Feature row. See migration V12.
  const [commitArtifacts, setCommitArtifacts] = useState<'inherit' | 'yes' | 'no'>('inherit');
  // Staged attachments — collected by the dropzone above the
  // description. Persisted by the parent via `feature_add_attachment`
  // after `start_feature` returns the new feature id; see the
  // `attachments` field on `onLaunch`.
  const [attachments, setAttachments] = useState<LaunchStageEntry[]>([]);
  const [attachmentError, setAttachmentError] = useState<string | null>(null);
  // Soft vision-support warning. Dismissable per-modal so the user
  // doesn't see it on every keystroke; resets on close.
  const [visionWarningDismissed, setVisionWarningDismissed] = useState(false);
  const titleRef = useRef<HTMLInputElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);

  // Remote execution (M6.1): "Where to run" + optional budget caps.
  // `machineId === ''` means "run here" (today's behavior); any other
  // value is a detached run, which is *always* unattended (a detached run
  // can't block on a human, so there is no attended mode to toggle).
  const [machines, setMachines] = useState<Machine[]>([]);
  const [machineId, setMachineId] = useState<string>('');
  const [maxCostUsd, setMaxCostUsd] = useState<string>('');
  const [maxWallClockMins, setMaxWallClockMins] = useState<string>('');

  // Detached — the run executes under `demeteo-runner` on the chosen
  // machine. The full launch surface (attachments, per-step overrides,
  // commit-artifacts, repo choice) ships in the RunSpec since M-E; the
  // one remaining asymmetry is that a detached run clones a single
  // repository, annotated on the repo picker below.
  const detached = machineId !== '';
  // Attached-remote is a project-level setting, not a per-run choice:
  // a machine-less launch on such a project executes over SSH with the
  // desktop app orchestrating. Stated in "Where to run" as a fact.
  const attachedRemote = computeType === 'remote';
  const remoteMachines = useMemo(
    () => machines.filter((m) => m.auth_type !== 'local'),
    [machines],
  );

  useEffect(() => {
    if (isOpen) {
      // Prefill from the inline composer, but only into still-empty
      // fields so a user editing in the modal is never clobbered.
      // Seeding `description` drives the modal's repo-chip inference.
      if (seedTitle && !title) {
        setTitle(seedTitle);
        setDescription(seedTitle);
      }
      if (seedAttachments && seedAttachments.length > 0 && attachments.length === 0) {
        setAttachments(seedAttachments);
      }
      setTimeout(() => titleRef.current?.focus(), 0);
    } else {
      // reset on close so the next open is clean
      setTitle('');
      setDescription('');
      setWorkflowId('');
      setAgentKind('');
      setModel('');
      setEffort('');
      setShowAdvanced(false);
      setCommitArtifacts('inherit');
      setSteps([]);
      setStepOverrides({});
      setLoopIterations('');
      setRepoOverride(null);
      setOriginSelection({ base: null, diffBase: null });
      setAttachments([]);
      setAttachmentError(null);
      setVisionWarningDismissed(false);
      setMachineId('');
      setMaxCostUsd('');
      setMaxWallClockMins('');
    }
  }, [isOpen, seedTitle, seedAttachments]);

  // Seed the workflow picker exactly once per open, and only once both inputs
  // the rule reads have answered.
  //
  // "Once" is a ref rather than a `!workflowId` guard because such a guard has
  // to read `workflowId`, which puts the effect's own output into its
  // dependency list: the effect then re-runs on every pick and snaps a
  // `defaultWorkflowId` launch back to the prop, leaving a picker the user
  // cannot change at all. Every milder version of that is the same bug — a
  // re-seed on a workflow-list refresh or a prop identity change overwrites a
  // choice already made, which is worse than starting on the wrong workflow.
  const seededRef = useRef(false);
  useEffect(() => {
    if (!isOpen) {
      seededRef.current = false;
      return;
    }
    if (seededRef.current || !workflowsLoaded || !projectDefaultLoaded) return;
    seededRef.current = true;
    setWorkflowId(
      resolveLaunchWorkflowId({
        workflows,
        requestedId: defaultWorkflowId,
        projectDefaultId: projectDefaultWorkflowId,
      }) ?? '',
    );
  }, [
    isOpen,
    workflows,
    workflowsLoaded,
    projectDefaultLoaded,
    projectDefaultWorkflowId,
    defaultWorkflowId,
  ]);

  // The dropzone can only observe paste events targeted at itself. Route
  // image files pasted into the title, description, or any other modal
  // descendant through the same launch staging helper, while leaving text
  // and HTML paste entirely to the browser. The listener deliberately
  // excludes the embedded dropzone because it already owns its own paste
  // handler and staging path.
  useEffect(() => {
    if (!isOpen) return;
    const modal = modalRef.current;
    if (!modal) return;
    let cancelled = false;

    const handlePaste = async (event: ClipboardEvent) => {
      const target = event.target;
      if (!(target instanceof Node) || !modal.contains(target)) return;
      if (target instanceof Element && target.closest('[data-attachment-dropzone]')) return;
      if (!event.clipboardData) return;

      const extraction = extractClipboardImageFiles(event.clipboardData);
      if (extraction.kind === 'none') {
        if (event.clipboardData.items.length !== 0) return;
        const recovery = await recoverClipboardImageFile();
        if (recovery.kind !== 'recovered') {
          if (!cancelled) {
            setAttachmentError(
              'This webview could not read image bytes from the clipboard. Save it and attach it, or try another clipboard source.',
            );
          }
          return;
        }
        event.preventDefault();
        try {
          const next = await stageBrowserFilesForLaunch([recovery.file], attachments);
          if (!cancelled) {
            setAttachments(next);
            setAttachmentError(null);
          }
        } catch (err) {
          if (!cancelled) setAttachmentError(formatError(err));
        }
        return;
      }
      if (extraction.kind === 'unavailable') {
        setAttachmentError(
          'The clipboard offered an image, but this webview could not access its file. Save it and attach it, or try another clipboard source.',
        );
        return;
      }

      event.preventDefault();
      try {
        const next = await stageBrowserFilesForLaunch(extraction.files, attachments);
        if (!cancelled) {
          setAttachments(next);
          setAttachmentError(null);
        }
      } catch (err) {
        if (!cancelled) setAttachmentError(formatError(err));
      }
    };

    modal.addEventListener('paste', handlePaste);
    return () => {
      cancelled = true;
      modal.removeEventListener('paste', handlePaste);
    };
  }, [isOpen, attachments]);

  // Fetch selectable workflows whenever the modal opens. Fetched here
  // rather than threaded through from the parent so the picker is never
  // empty regardless of which entry point opened the modal.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    setWorkflowsLoaded(false);
    (async () => {
      try {
        const list = await listWorkflows();
        if (!cancelled) setWorkflows(list ?? []);
      } catch (e) {
        console.warn('failed to load workflows for start-feature picker:', e);
        if (!cancelled) setWorkflows([]);
      } finally {
        if (!cancelled) setWorkflowsLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen]);

  // The project's stored launch default (audit F10). Read here rather than
  // threaded down as a prop: the modal already fetches its own workflow list
  // for the same reason, and an entry point that forgot to pass it would
  // silently fall back to a rule the user never chose. A failed read is the
  // unset path — the picker resolves without it and the launch is never
  // blocked on a setting.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    setProjectDefaultLoaded(false);
    (async () => {
      try {
        const settings = await getProposedStrategy(projectId);
        if (!cancelled) setProjectDefaultWorkflowId(settings?.default_workflow_id ?? null);
      } catch (e) {
        console.warn('failed to read the project default workflow:', e);
        if (!cancelled) setProjectDefaultWorkflowId(null);
      } finally {
        if (!cancelled) setProjectDefaultLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, projectId]);

  // Fetch remote machines whenever the modal opens (M6.1). Local runs
  // don't need this list, so it's fetched lazily rather than threaded
  // through from the parent.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    (async () => {
      try {
        const list = await listMachines();
        if (!cancelled) setMachines(list ?? []);
      } catch (e) {
        console.warn('failed to load machines for remote-run picker:', e);
        if (!cancelled) setMachines([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen]);

  // Load the agents configured/available on the target machine so the
  // agent pickers reflect reality. Re-fetched when the machine changes;
  // `''` (run here) resolves to the built-in local config via `'local'`.
  // `refresh: false` uses the cached availability probe — the Strategy
  // settings tab owns the expensive "Re-check" path.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    const mid = machineId || 'local';
    (async () => {
      try {
        const list = await getAgentConfigs(mid, false);
        if (!cancelled) setAgentConfigs(list ?? []);
      } catch (e) {
        console.warn('failed to load agent configs for start-feature picker:', e);
        if (!cancelled) setAgentConfigs([]);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, machineId]);

  // Load the selected workflow's steps so the user can override the agent /
  // model per step. Gate steps don't run an agent, so they're filtered out.
  useEffect(() => {
    if (!isOpen || !workflowId) {
      setSteps([]);
      setGraph(null);
      return;
    }
    // Clear before fetching, not only when the selection empties: otherwise
    // *switching* workflows leaves the previous one's shape on screen under
    // the new one's name until the fetch resolves — on the surface whose whole
    // job is "is this the pipeline I meant?".
    setSteps([]);
    setGraph(null);
    let cancelled = false;
    (async () => {
      try {
        const w = await getWorkflow(workflowId);
        if (cancelled) return;
        const rows: StepRow[] = (w.steps || [])
          .filter((s) => s.kind !== 'gate')
          .map((s) => ({ id: s.id, title: s.title, kind: s.kind }));
        setSteps(rows);
        // The graph the run will follow (P3.6). Fetched separately because the
        // override list is a flat list of agent steps and cannot show shape —
        // a fan-out or a gate is exactly what a launcher wants to confirm.
        // Best-effort: a failure leaves the preview off, never blocks launch.
        try {
          const def = await workflowVersionGraph(workflowId, w.version_id);
          if (!cancelled) setGraph(def);
        } catch {
          if (!cancelled) setGraph(null);
        }
      } catch (e) {
        console.warn('failed to load workflow steps for per-step overrides:', e);
        setSteps([]);
        setGraph(null);
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

  // Repos the run targets when the user hasn't overridden the selection:
  // the inferred ones, or all project repos when nothing was inferred.
  const autoRepoIds = useMemo(
    () => (inferredRepos.length > 0 ? inferredRepos.map((r) => r.id) : repositories.map((r) => r.id)),
    [inferredRepos, repositories],
  );
  // Single source of truth for what will actually run. An explicit
  // override wins; otherwise fall back to auto-detect.
  const selectedRepoIds = repoOverride ?? autoRepoIds;

  // Vision-capability check: shown as a soft warning when the user
  // attaches at least one image and the resolved model does NOT
  // advertise image support. The signal is the same one the agent
  // probe uses on the Rust side — see
  // `application::agent_probe::model_supports_images_by_name` and the
  // wrapper in `src/lib/agentModels.ts`. The banner is dismissable;
  // it never blocks launch (spec §0 decision #4).
  const hasImageAttachment = useMemo(
    () => attachments.some((a) => a.mime.startsWith('image/')),
    [attachments],
  );
  const modelSupportsImagesNow = useMemo(
    () => modelSupportsImagesByName(agentKind, model),
    [agentKind, model],
  );
  const showVisionWarning = hasImageAttachment && !modelSupportsImagesNow;

  // Agent choices for the pickers: the machine's enabled agents. Falls back
  // to the registered agent catalog if the probe returned nothing (e.g. an
  // unreachable machine) so the pickers are never empty. `available` drives
  // the "not installed" hint on each option.
  const agentOptions = useMemo(() => {
    const enabled = agentConfigs.filter((a) => a.enabled);
    if (enabled.length > 0) return enabled;
    return agentCatalog.map((a) => ({
      kind: a.kind,
      enabled: true,
      available: true,
      install_command: a.install_command,
      display_label: a.display_label,
    }));
  }, [agentConfigs, agentCatalog]);
  const agentKinds = useMemo(() => agentOptions.map((a) => a.kind), [agentOptions]);

  // The agent kinds any picker currently needs a model list for: the
  // "all steps" default plus every per-step override that names an agent.
  // A blank agent means "inherit" — there's no concrete agent to probe, so
  // its model select stays disabled (same as the Strategy tab).
  const neededModelKinds = useMemo(() => {
    const set = new Set<string>();
    if (agentKind) set.add(agentKind);
    for (const s of steps) {
      const k = stepOverrides[s.id]?.agent_kind;
      if (k) set.add(k);
    }
    return Array.from(set);
  }, [agentKind, steps, stepOverrides]);

  // Kinds already probed for the *current* machine. Held in a ref (not
  // state) so the fetch effect below can dedupe without depending on
  // `modelsByAgent`/`modelsLoading` — depending on the maps it writes would
  // retrigger the effect and cancel its own in-flight probe, leaving the
  // picker stuck on "Probing models…".
  const probedRef = useRef<Set<string>>(new Set());

  // Model lists are per-machine, so drop any cached lists (and the
  // probed-set) when the target machine changes or the modal reopens. This
  // effect is declared before the fetch effect so, on a machine switch, the
  // reset runs first and the fetch effect re-probes against the new machine.
  useEffect(() => {
    probedRef.current = new Set();
    setModelsByAgent({});
    setModelsLoading({});
  }, [machineId, isOpen]);

  // Lazily probe models for each needed agent kind against the target
  // machine (`''` → run here → `'local'`). `getAgentModels` dedupes and
  // caches by `(machine, agent)`, so re-fires here are cheap. Fire-and-
  // forget: `probedRef` guards against duplicate probes, and each probe
  // commits its own result independently.
  useEffect(() => {
    if (!isOpen) return;
    const mid = machineId || 'local';
    for (const kind of neededModelKinds) {
      if (probedRef.current.has(kind)) continue;
      probedRef.current.add(kind);
      setModelsLoading((prev) => ({ ...prev, [kind]: true }));
      getAgentModels(mid, kind)
        .then((list) =>
          setModelsByAgent((prev) => ({
            ...prev,
            [kind]: (list ?? []).map((m) => ({ value: m.value, name: m.name })),
          })),
        )
        .catch((e) => {
          console.warn('failed to probe models for', kind, e);
          setModelsByAgent((prev) => ({ ...prev, [kind]: [] }));
        })
        .finally(() => setModelsLoading((prev) => ({ ...prev, [kind]: false })));
    }
  }, [isOpen, machineId, neededModelKinds]);

  // Model list + loading flag for a given agent kind, shaped for
  // `HarnessModelPicker`. A blank kind has no models to show.
  const modelsFor = (kind: string): { models: ModelOption[]; loading: boolean } => {
    if (!kind) return { models: [], loading: false };
    return { models: modelsByAgent[kind] ?? [], loading: Boolean(modelsLoading[kind]) };
  };

  // Q26 / Q11 — detect repos already used by another active feature
  // (so we can warn the user before they kick off a parallel run).
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    (async () => {
      try {
        const active = await fetchActiveFeatures(projectId);
        if (cancelled) return;
        const usedRepos = new Set<string>();
        for (const f of active) {
          const fRepos = await getRepositoriesForProject(f.project_id);
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

  // Detached attachments are SFTP-spooled through the submit call, so
  // they're capped tighter than the local path's 100 MB (mirrors
  // MAX_DETACHED_ATTACHMENT_BYTES in commands/remote_runner.rs).
  const DETACHED_ATTACHMENT_CAP = 25 * 1024 * 1024;
  const oversizedForDetached = detached
    ? attachments.filter((a) => a.size > DETACHED_ATTACHMENT_CAP)
    : [];

  const canLaunch =
    title.trim().length > 0 &&
    description.trim().length > 0 &&
    workflowId !== '' &&
    oversizedForDetached.length === 0 &&
    (repositories.length === 0 || selectedRepoIds.length > 0);

  const launch = () => {
    if (!canLaunch) return;
    const targetRepos = selectedRepoIds;
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
        effort: v.effort || null,
      }))
      .filter((o) => o.agent_kind || o.model || o.effort);
    const loopArg = loopIterations.trim() ? parseInt(loopIterations, 10) : undefined;
    const budgetArg = maxBudgetUsd.trim() ? parseFloat(maxBudgetUsd) : undefined;
    const costArg = detached && maxCostUsd.trim() ? parseFloat(maxCostUsd) : undefined;
    const wallClockArg =
      detached && maxWallClockMins.trim() ? parseInt(maxWallClockMins, 10) : undefined;
    onLaunch({
      workflowId,
      title: title.trim(),
      description: description.trim(),
      agentKind: agentKind.trim() || undefined,
      model: model.trim() || undefined,
      effort: effort || undefined,
      targetRepos,
      commitArtifacts: commitArtifactsArg,
      loopIterations: Number.isFinite(loopArg as number) ? loopArg : undefined,
      maxBudgetUsd:
        Number.isFinite(budgetArg as number) && (budgetArg as number) > 0
          ? budgetArg
          : undefined,
      stepOverrides: overrides.length > 0 ? overrides : undefined,
      attachments: attachments.length > 0 ? attachments : undefined,
      machineId: machineId || undefined,
      // Detached runs are always unattended — they can't block on a human
      // (Demeteo may be closed). Never send `false` for a detached run.
      unattended: detached ? true : undefined,
      maxCostUsd: Number.isFinite(costArg as number) ? costArg : undefined,
      maxWallClockMins: Number.isFinite(wallClockArg as number) ? wallClockArg : undefined,
      ...runOriginArgs(originSelection),
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
      {/* Viewport-capped panel: the form body scrolls internally while the
          header and footer stay pinned, so expanding "Customize…" (per-step
          overrides) can never grow the modal past the screen. */}
      <div ref={modalRef} className="bg-[#0a0a0e] border border-white/10 rounded-2xl w-full max-w-2xl shadow-2xl overflow-hidden max-h-[85vh] flex flex-col">
        <div className="px-6 py-4 border-b border-white/5 flex justify-between items-center bg-[#050508] shrink-0">
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

        <div className="p-6 space-y-4 flex-1 min-h-0 overflow-y-auto">
          {/* Attachments (above the description per sub-3 brief).
              Drop, paste, or pick up to 10 files (100 MB each). The dropzone
              stages locally in `LaunchStageEntry[]` — see the
              `attachments` field on `onLaunch` for the commit step. */}
          <div data-attachment-dropzone>
            <div className="flex items-center gap-2 mb-1.5">
              <span className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                Attachments
              </span>
              <span className="text-[10px] font-mono text-slate-500">
                optional · referenced as [attachment -- &lt;name&gt;] in prompts
              </span>
            </div>
            <AttachmentDropzone
              mode="launch"
              label="Add files"
              stageEntries={attachments}
              onChangeStage={setAttachments}
              onError={setAttachmentError}
              maxChips={6}
            />
            {attachmentError && (
              <p role="alert" className="mt-1.5 text-[11px] font-mono text-ruby-200">
                {attachmentError}
              </p>
            )}
            {detached && attachments.length > 0 && (
              <p className={`text-[10px] font-mono mt-1.5 ${oversizedForDetached.length > 0 ? 'text-amber-300' : 'text-slate-500'}`}>
                {oversizedForDetached.length > 0
                  ? `Too large for a detached run (max 25 MB per file): ${oversizedForDetached.map((a) => a.name).join(', ')}`
                  : 'Copied to the machine before launch — max 25 MB per file on detached runs.'}
              </p>
            )}
          </div>

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
            <label
              htmlFor="start-feature-workflow"
              className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider"
            >
              Workflow
            </label>
            <select
              id="start-feature-workflow"
              value={workflowId}
              onChange={(e) => setWorkflowId(e.target.value)}
              className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-sm text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
            >
              {/* Named, because no rule picked one: without an option matching
                  the empty value the select renders blank, which reads as a
                  loading bug rather than as a question. */}
              {workflowId === '' && (
                <option value="" disabled>
                  {workflows.length === 0 ? 'No workflows available' : 'Choose a workflow…'}
                </option>
              )}
              {workflows.map((w) => (
                <option key={w.id} value={w.id}>
                  {w.name} (v{w.version})
                </option>
              ))}
            </select>
          </div>

          {/* Where to run (docs/REMOTE_EXECUTION.md M6.1). Surfaced
              beside the workflow instead of inside Customize: where a run
              executes is as fundamental as what it runs, and burying the
              remote/unattended entry point made it invisible. */}
          <div className="rounded-lg border border-white/10 bg-white/[0.02] p-3">
            <label className="flex items-center gap-1.5 text-[11px] font-mono text-slate-300 mb-1.5 uppercase tracking-wider">
              <Server className="w-3.5 h-3.5 text-cyan-400" />
              Where to run
            </label>
            {remoteMachines.length > 0 ? (
              <>
                <select
                  value={machineId}
                  onChange={(e) => setMachineId(e.target.value)}
                  className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50"
                >
                  <option value="">
                    {attachedRemote ? `Project machine — ${remoteHost ?? 'remote host'} (SSH)` : 'This machine'}
                  </option>
                  {remoteMachines.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name} — detached
                    </option>
                  ))}
                </select>
                {!machineId && attachedRemote && (
                  <p className="text-[10px] font-mono text-cyan-300/80 mt-1.5 leading-relaxed">
                    Attached — executes on {remoteHost ?? 'the project machine'} over SSH with Demeteo
                    orchestrating (project setting). Keep the app open while it runs.
                  </p>
                )}
                {machineId && (
                  <div className="mt-2 space-y-2.5 pl-3 border-l border-white/5">
                    <p className="text-[10px] font-mono text-cyan-300/80 leading-relaxed">
                      Detached — runs on{' '}
                      <span className="font-semibold">
                        {machines.find((m) => m.id === machineId)?.name ?? machineId}
                      </span>
                      ; you can close Demeteo and the run continues.
                    </p>
                    <div className="flex items-start gap-2 rounded-lg bg-cyan-500/[0.07] border border-cyan-500/20 px-3 py-2">
                      <MoonStar className="w-3.5 h-3.5 text-cyan-300 mt-0.5 shrink-0" />
                      <p className="text-[10px] font-mono text-slate-400 leading-relaxed">
                        <span className="text-cyan-200 font-semibold">Always unattended.</span>{' '}
                        A detached run can't wait on you, so review gates and merges to the
                        feature branch auto-approve. Merge-to-default and over-budget gates
                        still park for your decision — you'll find them under Runs. Set optional
                        caps below to bound spend and runtime.
                      </p>
                    </div>
                    <div className="grid grid-cols-2 gap-2">
                        <div>
                          <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                            Max cost (USD)
                          </label>
                          <input
                            type="number"
                            min={0}
                            step="0.01"
                            value={maxCostUsd}
                            onChange={(e) => setMaxCostUsd(e.target.value)}
                            placeholder="no cap"
                            className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                          />
                        </div>
                        <div>
                          <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                            Max wall-clock (min)
                          </label>
                          <input
                            type="number"
                            min={0}
                            value={maxWallClockMins}
                            onChange={(e) => setMaxWallClockMins(e.target.value)}
                            placeholder="no cap"
                            className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                          />
                        </div>
                    </div>
                  </div>
                )}
              </>
            ) : (
              <p className="text-[10px] font-mono text-slate-500 leading-relaxed">
                {attachedRemote
                  ? `Executes on ${remoteHost ?? 'the project machine'} over SSH (project setting).`
                  : 'This machine. Add a remote machine under Machines to run detached.'}
              </p>
            )}
          </div>

          {/* Target repositories (Q25 — repos inferred from the description
              pre-check; toggling switches to an explicit selection). Kept
              always-visible and interactive: which repos a run touches is
              fundamental, not an advanced tweak, so it doesn't live behind
              Customize. */}
          {repositories.length > 0 && (
            <div>
              <div className="flex items-center gap-2 mb-2">
                <GitBranch className="w-3.5 h-3.5 text-violet-400" />
                <span className="text-[11px] font-mono text-slate-400 uppercase tracking-wider">
                  Target repositories
                </span>
                <span className="text-[10px] font-mono text-slate-500">
                  {repoOverride === null ? '(auto-detected from description)' : '(custom selection)'}
                </span>
              </div>
              {detached && (
                <p className="text-[10px] font-mono text-amber-300 mb-2">
                  Detached runs clone a single repository — the first selected repo is used.
                </p>
              )}
              <div className="space-y-1">
                {repositories.map((r) => {
                  const checked = selectedRepoIds.includes(r.id);
                  const inUse = conflicts.has(r.id);
                  const toggle = () =>
                    setRepoOverride(
                      checked
                        ? selectedRepoIds.filter((id) => id !== r.id)
                        : [...selectedRepoIds, r.id],
                    );
                  return (
                    <label
                      key={r.id}
                      className="flex items-center gap-2 text-xs font-mono text-slate-300 cursor-pointer"
                      title={r.repo_path}
                    >
                      <input
                        type="checkbox"
                        checked={checked}
                        onChange={toggle}
                        className="accent-cyan-500"
                      />
                      <span className="truncate">{r.repo_path}</span>
                      {inUse && (
                        <span className="flex items-center gap-1 text-amber-300">
                          <AlertTriangle className="w-3 h-3" />
                          conflict
                        </span>
                      )}
                    </label>
                  );
                })}
              </div>
              {selectedRepoIds.length === 0 && (
                <p className="text-[10px] font-mono text-amber-300 mt-1.5">
                  Select at least one repository to launch.
                </p>
              )}
            </div>
          )}

          <OriginPicker
            projectId={projectId}
            repositoryId={selectedRepoIds[0] ?? null}
            value={originSelection}
            onChange={setOriginSelection}
          />

          {/* Customize (Q22: expand to full form) */}
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setShowAdvanced((v) => !v)}
              className="flex items-center gap-1.5 text-xs text-cyan-300 hover:text-cyan-200 transition-colors"
            >
              {showAdvanced ? <ChevronUp className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
              {showAdvanced ? 'Hide' : 'Customize…'}
            </button>
          </div>

          {showAdvanced && (
            <div className="space-y-3 pl-3 border-l border-white/5">
              <div>
                <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                  Default agent, model &amp; effort — all steps
                </label>
                <HarnessModelPicker
                  agentKinds={agentKinds}
                  models={modelsFor(agentKind).models}
                  modelsLoading={modelsFor(agentKind).loading}
                  agentKind={agentKind}
                  model={model}
                  effort={effort}
                  effortLevels={effortLevelsFor(agentCatalog, agentKind)}
                  onAgentKindChange={(k) => {
                    setAgentKind(k);
                    // The selected model belongs to the previous agent's
                    // namespace — clear it so we don't submit a mismatched pair.
                    // The effort ladder is canonical across agents, but the new
                    // agent may not accept the current rung, so clamp it down
                    // (or clear it) rather than keep a level it can't run.
                    setModel('');
                    setEffort((e) => reconcileEffort(e, effortLevelsFor(agentCatalog, k)));
                  }}
                  onModelChange={setModel}
                  onEffortChange={setEffort}
                  onClear={() => {
                    setAgentKind('');
                    setModel('');
                    setEffort('');
                  }}
                  agentPlaceholder="project default"
                  modelPlaceholder="Agent default model"
                  effortPlaceholder="project default"
                />
              </div>

              {/* The shape of the pipeline this launch will follow (P3.6).
                  The override list below is flat by nature, so a gate or a
                  fan-out is invisible there — this is where "did I pick the
                  reviewed pipeline?" gets answered before spending anything. */}
              {graph && graph.nodes.length > 0 && (
                <div>
                  <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                    Pipeline shape
                  </label>
                  <div className="rounded-lg border border-white/5 bg-black/20 p-3 max-h-48 overflow-y-auto">
                    <MiniGraph definition={graph} />
                  </div>
                </div>
              )}

              {/* Per-step agent/model overrides. Blank row = inherit the
                  default above → the workflow step → the project default. */}
              {steps.length > 0 && (
                <div>
                  <label className="block text-[11px] font-mono text-slate-400 mb-1.5 uppercase tracking-wider">
                    Per-step overrides (optional)
                  </label>
                  <div className="space-y-2.5">
                    {steps.map((s, i) => {
                      const ov = stepOverrides[s.id] || { agent_kind: '', model: '', effort: '' as const };
                      const setOv = (patch: Partial<{ agent_kind: string; model: string; effort: EffortLevel | '' }>) =>
                        setStepOverrides((prev) => {
                          const cur = prev[s.id] || { agent_kind: '', model: '', effort: '' as const };
                          return { ...prev, [s.id]: { ...cur, ...patch } };
                        });
                      const m = modelsFor(ov.agent_kind);
                      return (
                        <div key={s.id}>
                          <div
                            className="text-[11px] text-slate-400 font-mono mb-1 truncate"
                            title={s.title}
                          >
                            {i + 1}. {s.title}
                          </div>
                          <HarnessModelPicker
                            agentKinds={agentKinds}
                            models={m.models}
                            modelsLoading={m.loading}
                            agentKind={ov.agent_kind}
                            model={ov.model}
                            effort={ov.effort}
                            inheritedAgentKind={agentKind}
                            effortLevels={effortLevelsFor(agentCatalog, ov.agent_kind || agentKind)}
                            onAgentKindChange={(k) =>
                              setOv({
                                agent_kind: k,
                                model: '',
                                // Clamp the row's effort to what the new
                                // effective harness (this row's agent, else the
                                // inherited default) accepts.
                                effort: reconcileEffort(ov.effort, effortLevelsFor(agentCatalog, k || agentKind)),
                              })
                            }
                            onModelChange={(model) => setOv({ model })}
                            onEffortChange={(effort) => setOv({ effort })}
                            onClear={() => setOv({ agent_kind: '', model: '', effort: '' })}
                            agentPlaceholder="inherit"
                            modelPlaceholder="inherit"
                            effortPlaceholder="inherit"
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
                  Per-turn budget (optional)
                </label>
                <input
                  type="number"
                  min={0}
                  step={0.5}
                  value={maxBudgetUsd}
                  onChange={(e) => setMaxBudgetUsd(e.target.value)}
                  placeholder="blank = project default ($20)"
                  className="w-full bg-[#050508] border border-white/10 rounded-lg px-3 py-2 text-xs text-slate-200 font-mono focus:outline-none focus:border-cyan-500/50 placeholder-slate-600"
                />
                <p className="text-[10px] font-mono text-slate-500 mt-1.5 leading-relaxed">
                  Dollar ceiling per agent turn (<span className="font-mono">--max-budget-usd</span>);
                  the coding turn gets the full amount, shorter role turns a fraction. Anti-runaway
                  guard, not a whole-run cap.
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
                {workflowId === 'wf-starter-docs-update' && (
                  <p className="text-[10px] font-mono text-amber-300/80 mt-1.5 leading-relaxed">
                    For docs-update: the new doc body lands at the real <code>docs/…​</code> path the survey/gate approved; <code>{'{{report_dir}}'}</code> holds only the short change-summary report. Leave <em>Project default</em> (commit <code>false</code>) so the new doc reaches the PR while the summary stays out of it.
                  </p>
                )}
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

        <div className="px-6 py-4 border-t border-white/5 bg-[#050508] flex flex-col gap-3">
          {showVisionWarning && !visionWarningDismissed && (
            <div
              role="alert"
              className="flex items-start gap-2 px-3 py-2 rounded-lg border border-violet-500/40 bg-ruby-500/10 text-ruby-200"
            >
              <EyeOff className="w-4 h-4 mt-0.5 shrink-0 text-ruby-300" />
              <div className="flex-1 min-w-0 text-[11px] font-mono leading-snug">
                <span className="font-semibold">Model {model.trim() || '(unset)'} does not read images</span>
                <span className="text-ruby-200/80">
                  {' '}— attachments will be referenced as paths only and not inlined.
                </span>
              </div>
              <button
                type="button"
                onClick={() => setVisionWarningDismissed(true)}
                aria-label="Dismiss vision warning"
                className="shrink-0 text-ruby-200 hover:text-white transition-colors"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
        </div>
        <div className="px-6 py-4 border-t border-white/5 bg-[#050508] shrink-0 flex justify-between items-center">
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
