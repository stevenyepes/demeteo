// Runtime smoke tests for the ProjectRail sidebar.
//
// Spec finding C-4: the wizard entry from the project rail must sit
// alongside the existing `+` Bootstrap Project button in the header
// bar, labelled with a Sparkles icon, and route to `create-project`
// (the same wizard as the empty-state card's fourth tile).
//
// This test file verifies:
//
//   (a) The expanded header renders both the legacy Bootstrap Project
//       button AND a sibling Sparkles button titled "New from zero".
//
//   (b) Clicking the Sparkles button triggers `navigate` with
//       `{ kind: 'create-project' }` — NOT `new-project` and NOT
//       any other AppView variant.
//
//   (c) The collapsed rail also surfaces the same Sparkles button so
//       the wizard entry remains reachable while the sidebar is
//       minimised.
//
//   (d)/(e) `ProjectRail` auto-probes, at mount, only the loaded projects
//       whose `status` has an active pipeline (running/bootstrapping/
//       gated/active) and whose `liveness` is still `undefined` (never
//       persisted, see `types.ts`), driving 'checking' (pulsing) ->
//       'online'/'offline' as each `checkWorkspaceLiveness` call resolves
//       — in both the expanded row list and the collapsed avatar rail. An
//       `idle` project in the same load is *not* probed at startup.
//
//   (f) `LivenessDot` renders nothing for 'unknown'/absent, in
//       isolation from ProjectRail's auto-check effect.
//
//   (g) Selecting a workspace (currentProjectId changes) fires a fresh
//       probe for it unconditionally, even if its status has no active
//       pipeline and it was never probed at startup.
//
//   (h) A rapid re-selection (A -> B -> A) does not let A's first,
//       still-in-flight probe overwrite the result of A's second probe
//       once both resolve — A's final liveness reflects whichever probe
//       was *initiated* most recently.
//
// Mirrors the runtime-throws-on-failure pattern in
// `src/wizard.renderer.test.tsx`. Sections (d)-(h) need to await
// microtasks for the stubbed `checkWorkspaceLiveness` calls to settle,
// so — like `src/context/ProjectContext.test.tsx` and
// `src/lib/project.test.ts` — the whole suite runs inside an async
// `run()` and exports the resulting promise rather than a plain object.

import { act, create, type ReactTestInstance, type ReactTestRenderer } from 'react-test-renderer';
import { useEffect, type ReactElement } from 'react';

import ProjectRail from './ProjectRail';
import { LivenessDot } from './ui/StatusBadge';
import {
  NavigationProvider,
  ProjectProvider,
  UIStateProvider,
  useNavigation,
  useProject,
  useUIState,
} from '../context';
import type { AppView, Project } from '../types';

// ── Tauri IPC stub for `check_workspace_liveness` ──────────────────────
//
// `ProjectRail` probes any project whose `liveness` is `undefined` (see
// the effect in `ProjectRail.tsx`), so mounting it with unchecked
// projects calls the real `checkWorkspaceLiveness` ->
// `invoke('check_workspace_liveness', ...)`. Stub the IPC bridge
// (`@tauri-apps/api/core`'s `invoke` reads `window.__TAURI_INTERNALS__`
// directly) so those calls resolve/reject deterministically instead of
// hitting a real Tauri runtime. Mirrors `src/lib/project.test.ts`'s
// `installIpcStub`, extended to key resolutions per-project so a single
// mount can exercise "checking -> online" and "checking -> offline" side
// by side.
type LivenessResolution =
  | { kind: 'online' | 'offline'; checkedAt?: string }
  | { kind: 'pending' }; // never settles for the lifetime of the test

function installLivenessIpcStub(resolutions: Record<string, LivenessResolution>): { calls: string[] } {
  const calls: string[] = [];
  (globalThis as unknown as {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
  }).__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      if (cmd !== 'check_workspace_liveness') {
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }
      const projectId = (args as { projectId: string }).projectId;
      calls.push(projectId);
      const resolution = resolutions[projectId];
      if (!resolution || resolution.kind === 'pending') return new Promise(() => {});
      return Promise.resolve({
        project_id: projectId,
        liveness: resolution.kind,
        checked_at: resolution.checkedAt ?? '2026-07-12T00:00:00Z',
      });
    },
  };
  return { calls };
}

function uninstallLivenessIpcStub(): void {
  delete (globalThis as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
}

// ── Controllable IPC stub, for the stale-probe-cancellation test (h) ───
//
// `installLivenessIpcStub` resolves every call to a fixed, per-id outcome
// immediately. Testing AC4 (a superseded probe must not overwrite a newer
// one for the same id) requires holding individual calls open and
// resolving them out of order, so this variant hands back one deferred
// per `invoke` call instead.
interface DeferredLivenessCall {
  projectId: string;
  resolve: (liveness: 'online' | 'offline', checkedAt?: string) => void;
  reject: (err: unknown) => void;
}

function installControllableLivenessStub(): { calls: DeferredLivenessCall[] } {
  const calls: DeferredLivenessCall[] = [];
  (globalThis as unknown as {
    __TAURI_INTERNALS__: { invoke: (cmd: string, args: Record<string, unknown>) => Promise<unknown> };
  }).__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      if (cmd !== 'check_workspace_liveness') {
        return Promise.reject(new Error(`unexpected command: ${cmd}`));
      }
      const projectId = (args as { projectId: string }).projectId;
      return new Promise((resolvePromise, rejectPromise) => {
        calls.push({
          projectId,
          resolve: (liveness, checkedAt) =>
            resolvePromise({ project_id: projectId, liveness, checked_at: checkedAt ?? '2026-07-12T00:00:00Z' }),
          reject: (err) => rejectPromise(err),
        });
      });
    },
  };
  return { calls };
}

// Flush the microtask queue so `checkWorkspaceLiveness(...).then(...)`
// callbacks (and the `dispatch` calls inside them) run and land inside
// `act()`, matching React's requirement that state updates be wrapped.
async function flushMicrotasks(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function mount(element: ReactElement): ReactTestRenderer {
  let renderer: ReactTestRenderer | null = null;
  act(() => { renderer = create(element); });
  if (!renderer) throw new Error('ProjectRail renderer did not initialise');
  return renderer;
}

function findButtonByTitle(root: ReactTestInstance, title: string): ReactTestInstance | null {
  const buttons = root.findAll((node) => typeof node.type === 'string' && (node.type as string) === 'button');
  for (const btn of buttons) {
    if ((btn.props as { title?: string }).title === title) return btn;
  }
  return null;
}

function clickButton(btn: ReactTestInstance): void {
  const handler = (btn.props as { onClick?: () => void }).onClick;
  if (typeof handler !== 'function') {
    throw new Error(`ProjectRail: expected button to expose an onClick handler`);
  }
  act(() => { handler(); });
}

// `LivenessDot` (StatusBadge.tsx) always renders as a `w-1.5 h-1.5
// rounded-full` div — distinct from the `w-2 h-2` workflow-status dot
// StatusBadge renders for `p.status` right next to it — so it can be
// found by className regardless of where in the row/avatar it sits.
function findLivenessDots(root: ReactTestInstance): ReactTestInstance[] {
  return root.findAll((node) => {
    if (typeof node.type !== 'string' || node.type !== 'div') return false;
    const className = (node.props as { className?: string }).className;
    return typeof className === 'string' && className.includes('w-1.5 h-1.5 rounded-full');
  });
}

function makeProject(id: string, overrides: Partial<Project> = {}): Project {
  return {
    id,
    name: id,
    status: 'idle',
    repos: 0,
    nodes: 0,
    spend: 0,
    tokens: 0,
    ...overrides,
  };
}

// Fixture for the liveness-transition tests below: every project starts
// unchecked (`liveness: undefined`), matching what LOAD_PROJECTS hands the
// app in production (liveness is never persisted — see `types.ts`). Fixed
// order + distinct ids let each project's terminal state be steered
// independently via `installLivenessIpcStub` and matched by index below.
// The first three carry an active-pipeline `status` so the startup sweep
// (AC1) probes them; `p-idle-unprobed` carries `status: 'idle'` and must
// be skipped by that same sweep.
const livenessFixture: Project[] = [
  makeProject('p-goes-online', { name: 'Online Co', status: 'running' }),
  makeProject('p-goes-offline', { name: 'Offline Co', status: 'bootstrapping' }),
  makeProject('p-stays-checking', { name: 'Checking Co', status: 'active' }),
  makeProject('p-idle-unprobed', { name: 'Idle Co', status: 'idle' }),
];

// Loads `livenessFixture` into ProjectContext on mount, then renders
// ProjectRail (expanded by default).
function LoadLivenessFixture(): ReactElement {
  const { dispatch } = useProject();
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: livenessFixture, reposByProject: {} });
  }, [dispatch]);
  return <ProjectRail />;
}

// Same, but also collapses the sidebar so ProjectRail renders the
// collapsed avatar branch.
function LoadLivenessFixtureCollapsed(): ReactElement {
  const { dispatch } = useProject();
  const { uiDispatch } = useUIState();
  useEffect(() => {
    dispatch({ type: 'LOAD_PROJECTS', projects: livenessFixture, reposByProject: {} });
    uiDispatch({ type: 'TOGGLE_SIDEBAR' });
  }, [dispatch, uiDispatch]);
  return <ProjectRail />;
}

// Capture component reads the active view out of NavigationContext
// after every render. The probe lives inside the same provider tree
// as ProjectRail so the click on the rail button is observed.
function CaptureView(props: { holder: { current: AppView | null } }): ReactElement {
  const { view } = useNavigation();
  props.holder.current = view;
  return <></>;
}

// Collapse the sidebar once on mount so the rail renders the
// collapsed branch. Lives inside UIStateProvider so the rail sees the
// toggled state.
function CollapseOnMount(): ReactElement {
  const { uiDispatch } = useUIState();
  useEffect(() => { uiDispatch({ type: 'TOGGLE_SIDEBAR' }); }, [uiDispatch]);
  return <ProjectRail />;
}

async function run() {
  // ── (a) Expanded header: Bootstrap + Sparkles buttons coexist ─────────

  const expandedTree = mount(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <ProjectRail />
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  const bootstrapBtn = findButtonByTitle(expandedTree.root, 'Bootstrap Project');
  if (!bootstrapBtn) {
    throw new Error('ProjectRail (expanded): Bootstrap Project button must remain in the header');
  }

  const newFromZeroBtn = findButtonByTitle(expandedTree.root, 'New from zero');
  if (!newFromZeroBtn) {
    throw new Error('ProjectRail (expanded): "New from zero" Sparkles button missing from the header');
  }

  expandedTree.unmount();

  // ── (b) Clicking Sparkles routes to create-project ────────────────────

  const holder: { current: AppView | null } = { current: null };

  const clickTree = mount(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <ProjectRail />
          <CaptureView holder={holder} />
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  const newFromZeroBtnClick = findButtonByTitle(clickTree.root, 'New from zero');
  if (!newFromZeroBtnClick) {
    throw new Error('ProjectRail (expanded, second mount): "New from zero" Sparkles button missing');
  }

  const beforeView = holder.current;
  clickButton(newFromZeroBtnClick);
  const afterView = holder.current;

  if (!afterView) {
    throw new Error('ProjectRail: navigation context view should be set after click');
  }
  if (afterView.kind !== 'create-project') {
    throw new Error(
      `ProjectRail: clicking "New from zero" must navigate to 'create-project'; got '${afterView.kind}' ` +
      `(before-click view was '${beforeView?.kind ?? 'null'}'). The legacy 'new-project' route is forbidden here.`,
    );
  }

  clickTree.unmount();

  // ── (c) Collapsed rail also exposes the Sparkles button ───────────────

  const collapsedTree = mount(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <CollapseOnMount />
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  const collapsedSparkles = findButtonByTitle(collapsedTree.root, 'New from zero');
  if (!collapsedSparkles) {
    throw new Error('ProjectRail (collapsed): "New from zero" Sparkles button must also be reachable when the sidebar is collapsed');
  }

  // And clicking it from the collapsed branch also routes to
  // create-project.
  const collapsedHolder: { current: AppView | null } = { current: null };
  collapsedTree.unmount();

  const collapsedClickTree = mount(
    <NavigationProvider>
      <ProjectProvider>
        <UIStateProvider>
          <CollapseOnMount />
          <CaptureView holder={collapsedHolder} />
        </UIStateProvider>
      </ProjectProvider>
    </NavigationProvider>,
  );

  const collapsedSparklesClick = findButtonByTitle(collapsedClickTree.root, 'New from zero');
  if (!collapsedSparklesClick) {
    throw new Error('ProjectRail (collapsed, second mount): "New from zero" Sparkles button missing');
  }
  clickButton(collapsedSparklesClick);
  if (collapsedHolder.current?.kind !== 'create-project') {
    throw new Error(
      `ProjectRail (collapsed): clicking "New from zero" must navigate to 'create-project'; ` +
      `got '${collapsedHolder.current?.kind ?? 'null'}'`,
    );
  }

  collapsedClickTree.unmount();

  // ── (d) Liveness dot: startup sweep is scoped to active-pipeline projects ──
  //
  // `ProjectRail` probes, at mount, only the loaded projects whose `status`
  // has an active pipeline and whose `liveness` is still `undefined` (see
  // the effect added alongside the trigger wiring). This exercises the
  // full lifecycle: 'checking' (pulsing) synchronously on load, then
  // 'online'/'offline' once each stubbed probe resolves — a project whose
  // probe never resolves keeps pulsing. `p-idle-unprobed` (status 'idle')
  // must not be probed at all (AC1).

  {
    const { calls } = installLivenessIpcStub({
      'p-goes-online': { kind: 'online' },
      'p-goes-offline': { kind: 'offline' },
      'p-stays-checking': { kind: 'pending' },
    });

    const livenessTree = mount(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <LoadLivenessFixture />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

    if (calls.length !== 3 || new Set(calls).size !== 3) {
      throw new Error(
        `ProjectRail: expected exactly one checkWorkspaceLiveness call per active-pipeline project, got ${JSON.stringify(calls)}`,
      );
    }
    if (calls.includes('p-idle-unprobed')) {
      throw new Error(
        `ProjectRail: an 'idle' project (no active pipeline) must not be probed at startup, but calls included it: ${JSON.stringify(calls)}`,
      );
    }

    // Immediately after load, before any probe resolves, every project
    // must show a pulsing 'checking' dot.
    const checkingDots = findLivenessDots(livenessTree.root);
    if (checkingDots.length !== 3) {
      throw new Error(`ProjectRail: expected 3 'checking' dots immediately after load, got ${checkingDots.length}`);
    }
    for (const dot of checkingDots) {
      const className = (dot.props as { className: string }).className;
      if (!className.includes('animate-pulse')) {
        throw new Error(`ProjectRail: dot must pulse while liveness='checking', got className='${className}'`);
      }
    }

    // The workflow-status dot (StatusBadge on p.status) is unaffected —
    // distinct sizing (w-2 h-2, not w-1.5 h-1.5) keeps it from colliding
    // with the liveness dot.
    const workflowDots = livenessTree.root.findAll((node) => {
      if (typeof node.type !== 'string' || node.type !== 'div') return false;
      const className = (node.props as { className?: string }).className;
      return typeof className === 'string' && className.includes('w-2 h-2 rounded-full');
    });
    if (workflowDots.length !== livenessFixture.length) {
      throw new Error(
        `ProjectRail: expected one workflow-status dot per project (${livenessFixture.length}), got ${workflowDots.length}`,
      );
    }

    // Let the stubbed probes resolve.
    await flushMicrotasks();

    const settledDots = findLivenessDots(livenessTree.root);
    if (settledDots.length !== 3) {
      throw new Error(`ProjectRail: expected 3 liveness dots after probes settle, got ${settledDots.length}`);
    }
    const [onlineDot, offlineDot, stillCheckingDot] = settledDots;
    const onlineClass = (onlineDot.props as { className: string }).className;
    const offlineClass = (offlineDot.props as { className: string }).className;
    const stillCheckingClass = (stillCheckingDot.props as { className: string }).className;

    if (!onlineClass.includes('emerald') || onlineClass.includes('animate-pulse')) {
      throw new Error(`ProjectRail: resolved liveness='online' must render a steady emerald dot, got className='${onlineClass}'`);
    }
    if (!offlineClass.includes('ruby') || offlineClass.includes('animate-pulse')) {
      throw new Error(`ProjectRail: resolved liveness='offline' must render a steady ruby/muted dot, got className='${offlineClass}'`);
    }
    if (!stillCheckingClass.includes('animate-pulse')) {
      throw new Error(`ProjectRail: a probe that hasn't resolved yet must keep pulsing, got className='${stillCheckingClass}'`);
    }

    livenessTree.unmount();
    uninstallLivenessIpcStub();
  }

  // ── (e) Liveness dot: collapsed rail avatars mirror the same transitions ──

  {
    const { calls } = installLivenessIpcStub({
      'p-goes-online': { kind: 'online' },
      'p-goes-offline': { kind: 'offline' },
      'p-stays-checking': { kind: 'pending' },
    });

    const livenessCollapsedTree = mount(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <LoadLivenessFixtureCollapsed />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

    if (calls.length !== 3) {
      throw new Error(`ProjectRail (collapsed): expected one checkWorkspaceLiveness call per project, got ${calls.length}`);
    }

    await flushMicrotasks();

    const collapsedLivenessDots = findLivenessDots(livenessCollapsedTree.root);
    if (collapsedLivenessDots.length !== 3) {
      throw new Error(
        `ProjectRail (collapsed): expected 3 liveness dots on avatars, got ${collapsedLivenessDots.length}`,
      );
    }

    const collapsedOnlineClass = (collapsedLivenessDots[0].props as { className: string }).className;
    if (!collapsedOnlineClass.includes('emerald')) {
      throw new Error(
        `ProjectRail (collapsed): resolved liveness='online' avatar dot must use an emerald tone, got className='${collapsedOnlineClass}'`,
      );
    }
    const collapsedOfflineClass = (collapsedLivenessDots[1].props as { className: string }).className;
    if (!collapsedOfflineClass.includes('ruby')) {
      throw new Error(
        `ProjectRail (collapsed): resolved liveness='offline' avatar dot must use a ruby tone, got className='${collapsedOfflineClass}'`,
      );
    }

    livenessCollapsedTree.unmount();
    uninstallLivenessIpcStub();
  }

  // ── (f) LivenessDot: 'unknown'/absent renders no dot ────────────────────
  //
  // Decoupled from ProjectRail's auto-check effect above (which makes an
  // `undefined` liveness a fleeting, same-tick state once a project is
  // loaded) — pins the render contract directly: given 'unknown' or no
  // reading at all, LivenessDot must render nothing.

  {
    let renderer: ReactTestRenderer | null = null;
    act(() => { renderer = create(<LivenessDot />); });
    if (!renderer) throw new Error('LivenessDot: renderer did not initialise (liveness=undefined)');
    if ((renderer as ReactTestRenderer).toJSON() !== null) {
      throw new Error('LivenessDot: liveness=undefined must render nothing (no placeholder dot)');
    }
    (renderer as ReactTestRenderer).unmount();
  }
  {
    let renderer: ReactTestRenderer | null = null;
    act(() => { renderer = create(<LivenessDot liveness="unknown" />); });
    if (!renderer) throw new Error('LivenessDot: renderer did not initialise (liveness=unknown)');
    if ((renderer as ReactTestRenderer).toJSON() !== null) {
      throw new Error("LivenessDot: liveness='unknown' must render nothing (no placeholder dot)");
    }
    (renderer as ReactTestRenderer).unmount();
  }

  // ── (g) Selecting a workspace probes it on-demand, unconditionally ──────
  //
  // `p-on-demand` has status 'idle' (no active pipeline), so the startup
  // sweep (AC1) skips it. Selecting it (SET_CURRENT) must still fire a
  // fresh probe for it (AC3) — the active-pipeline filter only governs the
  // startup sweep, never the on-demand check.

  {
    const onDemandProject = makeProject('p-on-demand', { name: 'On Demand Co', status: 'idle' });
    const { calls } = installLivenessIpcStub({
      'p-on-demand': { kind: 'online' },
    });

    function LoadAndSelectOnDemand(): ReactElement {
      const { dispatch } = useProject();
      useEffect(() => {
        dispatch({ type: 'LOAD_PROJECTS', projects: [onDemandProject], reposByProject: {} });
        dispatch({ type: 'SET_CURRENT', id: 'p-on-demand' });
      }, [dispatch]);
      return <ProjectRail />;
    }

    const onDemandTree = mount(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <LoadAndSelectOnDemand />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

    if (calls.length !== 1 || calls[0] !== 'p-on-demand') {
      throw new Error(
        `ProjectRail: selecting an idle (non-active-pipeline) project must probe it on-demand, got calls=${JSON.stringify(calls)}`,
      );
    }

    const checkingDot = findLivenessDots(onDemandTree.root)[0];
    if (!checkingDot || !(checkingDot.props as { className: string }).className.includes('animate-pulse')) {
      throw new Error('ProjectRail: on-demand probe must dispatch a pulsing "checking" dot synchronously');
    }

    await flushMicrotasks();

    const onlineDot = findLivenessDots(onDemandTree.root)[0];
    const onlineClass = (onlineDot.props as { className: string }).className;
    if (!onlineClass.includes('emerald') || onlineClass.includes('animate-pulse')) {
      throw new Error(`ProjectRail: on-demand probe must resolve to a steady emerald dot, got className='${onlineClass}'`);
    }

    onDemandTree.unmount();
    uninstallLivenessIpcStub();
  }

  // ── (h) Stale-probe cancellation on rapid A -> B -> A switch (AC4) ──────
  //
  // A's first probe is left in flight (never resolved) while the user
  // switches to B and back to A, firing a second probe for A. The second
  // probe resolves first; then the stale first probe resolves with a
  // conflicting result. The stale result must be dropped — A's dot must
  // keep reflecting the most recently *initiated* probe's outcome.

  {
    const { calls } = installControllableLivenessStub();

    const staleFixture: Project[] = [
      makeProject('p-a', { name: 'A Co', status: 'idle' }),
      makeProject('p-b', { name: 'B Co', status: 'idle' }),
    ];
    const selectHolder: { select: ((id: string) => void) | null } = { select: null };

    function LoadStaleFixture(): ReactElement {
      const { dispatch } = useProject();
      useEffect(() => {
        dispatch({ type: 'LOAD_PROJECTS', projects: staleFixture, reposByProject: {} });
      }, [dispatch]);
      useEffect(() => {
        selectHolder.select = (id: string) => dispatch({ type: 'SET_CURRENT', id });
      });
      return <ProjectRail />;
    }

    const staleTree = mount(
      <NavigationProvider>
        <ProjectProvider>
          <UIStateProvider>
            <LoadStaleFixture />
          </UIStateProvider>
        </ProjectProvider>
      </NavigationProvider>,
    );

    if (!selectHolder.select) throw new Error('ProjectRail: setup failed to expose a SET_CURRENT dispatcher');

    act(() => { selectHolder.select!('p-a'); }); // call #0: A, generation 1
    act(() => { selectHolder.select!('p-b'); }); // call #1: B
    act(() => { selectHolder.select!('p-a'); }); // call #2: A, generation 2 (supersedes call #0)

    if (calls.length !== 3 || calls[0].projectId !== 'p-a' || calls[1].projectId !== 'p-b' || calls[2].projectId !== 'p-a') {
      throw new Error(`ProjectRail: expected probes [p-a, p-b, p-a] for the A->B->A switch, got ${JSON.stringify(calls.map(c => c.projectId))}`);
    }

    // Resolve A's latest (second) probe first.
    await act(async () => {
      calls[2].resolve('online', '2026-07-12T02:00:00Z');
      await Promise.resolve();
      await Promise.resolve();
    });

    let aClass = (findLivenessDots(staleTree.root)[0].props as { className: string }).className;
    if (!aClass.includes('emerald') || aClass.includes('animate-pulse')) {
      throw new Error(`ProjectRail: A's latest probe must settle it to a steady emerald dot, got className='${aClass}'`);
    }

    // Now resolve A's stale first probe with a conflicting result — must
    // be ignored since a newer probe for 'p-a' has since been initiated.
    await act(async () => {
      calls[0].resolve('offline', '2026-07-12T01:00:00Z');
      await Promise.resolve();
      await Promise.resolve();
    });

    aClass = (findLivenessDots(staleTree.root)[0].props as { className: string }).className;
    if (!aClass.includes('emerald') || aClass.includes('animate-pulse')) {
      throw new Error(
        `ProjectRail: a stale, superseded probe must not overwrite a newer result for the same project; expected A to remain online (emerald), got className='${aClass}'`,
      );
    }

    staleTree.unmount();
    uninstallLivenessIpcStub();
  }

  return {
    expanded: {
      hasBootstrapButton: true,
      hasSparklesButton: true,
      sparklesRoutesTo: 'create-project',
    },
    collapsed: {
      hasSparklesButton: true,
      sparklesRoutesTo: 'create-project',
    },
    liveness: {
      startupSweepScopedToActivePipeline: true,
      idleProjectNotProbedAtStartup: true,
      checkingRendersPulsingDot: true,
      checkingTransitionsToOnline: true,
      checkingTransitionsToOffline: true,
      unresolvedProbeStaysChecking: true,
      collapsedAvatarsMirrorTransitions: true,
      unknownRendersNoDot: true,
      workflowDotUnaffected: true,
      onDemandProbeOnSelectionChange: true,
      staleProbeCancellationOnRapidReselect: true,
    },
  } as const;
}

export const projectRailTestResults = run();
