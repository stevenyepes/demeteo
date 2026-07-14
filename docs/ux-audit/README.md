# UX & Product Audit — July 2026

A full pass over the Demeteo frontend (`src/`) cross-checked against the Tauri command
surface (`src-tauri/src/lib.rs`, `commands/*`), the event bus
(`adapters/tauri_ui/notification.rs`), and the existing design docs
(`docs/UX_JOURNEYS.md`, `src/docs/*.md`).

| File | Contents |
|------|----------|
| [`user-journeys.md`](user-journeys.md) | The key user journeys **as actually implemented**, with entry points, screens, and per-journey friction notes. |
| [`findings.md`](findings.md) | Numbered findings F1–F49: bugs, broken UX, inconsistencies, and improvement opportunities, each with severity and `file:line` references. F1–F33 are from the first pass; F34–F49 from the second (editor, machines, terminal, settings tabs, wizards). |

> **Roadmap status:** these findings are scheduled in the
> [H2 2026 roadmap](../roadmap/03-roadmap-6-months.md) as **Theme F — UX
> Quality & Trust**: P1s → epic UX1 (v1.1, Sep), P2s → UX2 (v1.2, Nov),
> P3s + opportunities → UX3 (v1.3, Jan, scoped at the M4 review).

## Headline results

- **Every frontend `invoke()` call maps to a registered Tauri command** — no missing/renamed
  command bugs were found. The problems are all one level up: UI that collects input and
  silently drops it, labels that don't match behavior, and parallel copies of the same
  UI drifting apart.
- **9 high-severity findings** (F1–F6, F34–F36): "Stop Step" cancels the whole feature;
  repository targeting is collected in three places and never sent to the backend; the
  launch-modal conflict detector marks *every* repo as conflicted whenever any feature is
  active; the in-app documentation is unreadable in production builds; retrying a failed
  project bootstrap creates duplicate project records; **Monaco is fetched from a CDN at
  runtime**, so Browse Code / artifact previews / gate review break offline; **Escape is
  double-handled by every dialog** (closing a modal also navigates the view underneath);
  and **~3,200 lines of dead parallel implementations** ship in the bundle, including an
  entire second create-from-zero wizard.
- **The keyboard-shortcut system has three sources of truth** (declarative registry,
  runtime dispatcher, markdown doc) and all three disagree. The help overlay component
  (`ShortcutHelp` + `ShortcutsContext`) is dead code — it is never mounted. The registry
  also promises mouse-back suppression while modals are open; no such suppression exists.
- **Built capability with no UI**: pause/resume, workflow import, workflow version
  history, post-launch attachments, post-sync re-validation, and the `conflict_detected`
  event all exist as registered backend commands/events with no frontend surface.
- **Doc drift**: `docs/UX_JOURNEYS.md` promises Pause/Resume actions, a full-screen gate
  takeover, and default conflict policy `auto_agent`; the implementation has none of the
  first, a modal for the second, and `always_gate` for the third.

## Method

**First pass** (F1–F33), read in full: `App.tsx`, all contexts, `ProjectHome`,
`FeatureDetail`, `StartFeatureModal`, `GateView`, `WorkflowList`,
`ProvidersPage`/`ProviderSettings`, `NewProjectView`, `PreferencesScreen`,
`RemoteRunInbox` (since deleted — see `REMOTE_EXECUTION_PLAN.md` M6.2 amendment; findings
F14 and F31 were resolved by that deletion), `TopBar`, `ProjectRail`, `NotificationBell`,
`EmptyStateCard`, `CommandPalette`, `DocsPanel`, `ProjectSettingsShell`, `lib/shortcuts.ts`,
`hooks/useKeyboardShortcuts.ts`, `lib/features.ts`, `lib/utils.ts`,
`lib/modelImageSupport.ts`. Backend spot-checks: command registrations,
`start_feature`/`feature_cancel` signatures, `get_active` SQL, status vocabulary, and
the event-emission adapter.

**Second pass** (F34–F49), read in full: `WorkflowEditor`, `MachinesView`, `EnvModal`,
`TerminalWindow`, `AgentTerminalDrawer`, `CodeEditorView`, `ArtifactViewer`,
`ui/Modal`, `PromptDialog`, `CommandSelector`, `MemoryAgentSettings`,
`ProjectSettingsContext`, `GeneralTab`, `StrategyTab`, `OverridesTab`,
`wizard/CreateProjectWizard`, `hooks/useMouseNavigation`, `hooks/useTauriEvent`,
`lib/errorBus`, `lib/errors`, `lib/featureSync`, `lib/agentModels`; dead-code
reachability checks for `CreateFromZeroWizard`/`ui/CreateZero*`,
`useCreateProjectWizard`, `lib/createProject`, `CommandSelector`,
`TerminalStatusOverlay`; Monaco loader configuration check across `vite.config.ts` and
`package.json`.
