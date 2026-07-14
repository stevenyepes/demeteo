# Key User Journeys — as implemented (July 2026)

This is the *as-built* companion to `docs/UX_JOURNEYS.md` (the designer-facing spec).
Where the two diverge, the divergence is called out inline and cross-referenced to
[`findings.md`](findings.md) (`F<n>`).

Navigation model: a single-window shell (`App.tsx`) with a view stack
(`NavigationContext`, back/forward + mouse XButton support), a left project rail, a top
bar, and overlays (command palette, docs panel, start-feature modal, gate modal).

---

## J1 — First run & onboarding

**Entry:** app launch with an empty DB → `empty-state` view (`EmptyStateCard`).

1. User sees four tiles — Connect Providers, Sync Worktrees, Deploy Agents, Create from
   Scratch — plus the primary "Try a sample project" button.
2. "Try a sample project" → `seed_sample_project` → project appears, rail selects it,
   navigate to Project Home.
3. "Create from Scratch" → the 7-step Create-Project wizard (J3b).

**Friction:** two tile labels don't match their destinations — "Sync Worktrees" opens the
*new project* form and "Deploy Agents" opens the *workflow list* (F22). From the workflow
list a user with zero projects can press ▶ Run and nothing happens (F15).

## J2 — Connecting a provider

**Entry:** TopBar → Providers, empty-state tile, or Settings → Providers tab (which just
links back to the Providers page).

1. `ProvidersPage` lists connected instances; "Connect Provider" opens `ProviderSettings`.
2. User picks GitHub/GitLab, enters alias, optional host, PAT → `connect_provider_instance`
   validates the PAT, fetches username + avatar → success state → card appears.
3. Edit re-opens the same modal; delete warns only if projects reference the provider.

**Friction:** the required "Provider Name / Alias" is never persisted — the backend call
doesn't receive it and reloads display the provider `kind` instead (F8). Editing forces a
silent PAT re-entry and actually re-runs *connect*, not update (F8). Deleting an unused
provider has no confirmation at all (F23).

## J3a — Project bootstrap from existing repos

**Entry:** rail "+" button, `Cmd/Ctrl+N`, or command palette → `NewProjectView`.

1. Name the project, pick Local vs Remote SSH (machine dropdown + test-connection +
   key-passphrase for key-auth machines), select repos from connected providers.
2. "Initialize & Analyze" → `create_project` → `bootstrap_project` (clone + strategy
   detection) → strategy proposal form (default branch, branch prefix, test command,
   conflict policy, feature lifecycle, detected PR template).
3. "Approve & Build Workspace" → `save_project_settings` → navigate to Project Home.

**Friction:** "Retry Build" after a failed bootstrap calls `create_project` again and
creates a duplicate project row (F6). The project object pushed into UI state omits
`compute_type`/`remote_host`, so a remote project doesn't show its Terminal tab until
restart (F7). Abandoning at the proposal step leaves an orphaned half-bootstrapped
project that only appears after restart (F6). The proposal form defaults conflict policy
to `always_gate`, while the UX spec says `auto_agent` is the default (F28).

## J3b — Create a project from zero (wizard)

**Entry:** rail ✨ button, empty-state "Create from Scratch", or the link inside
`NewProjectView`.

Seven one-decision screens (Name → Provider → Group → Machine → Agent → Model →
Description), state machine driven by the Rust side
(`begin_create_project` / `submit_create_project_step` / `go_back_create_project`);
commit creates the repo on the provider and auto-launches the starter workflow.

**Friction:** the Agent step offers `antigravity`, which the README declares unsupported
and other surfaces filter out (F21). On launch, the project is added to the rail under
the *feature title* instead of the project name until the next restart (F41). A complete
dead duplicate of this wizard (`CreateFromZeroWizard` + 14 `ui/CreateZero*` files) still
ships in the bundle (F36).

## J4 — Project Home (control center)

**Entry:** selecting a project in the rail / `Cmd+1..9`.

- Header: project name, settings gear, fleet/token telemetry.
- "Start a new Feature Pipeline" inline composer: attachments dropzone, description
  textarea, smart workflow inference from keywords, suggested repo chips, Customize
  drawer (workflow, target repos, agent/model override, step-timeline preview).
- "Start a coding session" card → interactive agent terminal drawer in a repo.
- "Active Running Pipelines" list → click-through to Feature Detail.
- Remote projects additionally get a Pipelines/Terminal tab bar.

**Friction:** the header hardcodes "Connected via GitHub Enterprise • Default Workflow:
Standard Feature Pipeline" for every project (F10). Repo chips/checkboxes are collected
but never sent — `start_feature` has no repo parameter (F2). The pipeline list renders
every non-archived feature (including failed/completed ones) as a pulsing "RUNNING FLEET"
(F9). The list shows duration/tokens but not the cost telemetry the spec promises. The
rail's "nodes" metric is a hardcoded 4-or-8 written on settings save, not a measurement
(F42).

## J5 — Starting a feature

**Entry:** Project Home composer, `Cmd/Ctrl+T`, `Cmd/Ctrl+Shift+N`, or Workflow list ▶
(these open `StartFeatureModal`; the composer is a separate parallel implementation).

1. Attachments → Title → Description → Workflow picker.
2. Auto-inferred repo chips with "conflict" badges for repos already in use.
3. Customize: default agent/model (free text), remote machine picker + readiness probe +
   unattended toggle + budget caps, per-step agent/model overrides, loop iterations,
   commit-artifacts policy.
4. Launch → local: `start_feature` → navigate to Feature Detail; remote:
   `remote_submit_run` → confirmation dialog pointing at the Return Inbox.

**Friction:** conflict badges fire for *all* repos whenever *any* feature is active (F3).
`targetRepos` is computed and dropped (F2), and the promised "edit in Customize" repo
editor does not exist in this modal (F2). Remote launches silently discard attachments,
per-step overrides, and the commit-artifacts choice (F13). Leaving the model blank while
attaching an image always shows the "model does not read images" warning even when the
project default is vision-capable (F20). Two parallel start-feature UIs (composer vs
modal) already diverge in behavior (F28).

## J6 — Monitoring a feature (Feature Detail)

**Entry:** pipeline list click, `Cmd/Ctrl+G` cycling, post-launch navigation, or a
`gate_required` event auto-navigating.

- Header: status chip (derived from step states), duration/cost/tokens/cache telemetry,
  actions: Code with Agent, Browse Code (read-only Monaco editor view), Cancel Feature,
  and for terminal states Sync with main / Publish MR / Cleanup.
- Timeline of step cards: status icon, retry count, per-step cost/tokens/duration,
  artifact chips (opens the right-hand `ArtifactViewer` pane), live agent stream toggle,
  Stop Step, Retry Step with harness/model override, Replay-from-step modal.
- Banners: awaiting-MR nudge, sync outcome (ok/conflict/resolved/failed with
  "Resolve with agent"), MR state row with refresh.
- Attachments strip with click-to-preview modal.

**Friction:** "Stop Step" cancels the entire feature (F1). The spec's Pause/Resume
actions exist as registered backend commands but have no UI (F12). `step_progress`
events clobber the pipeline-total cost with a single step's cost until the next reload
(F19). Worktree-ref artifacts are misclassified as plain JSON and their "Open in
Editor" flow is unreachable (F18, F46). Artifact previews and Browse Code depend on
CDN-loaded Monaco — broken offline (F34). Attachments are read-only after launch;
there's no way to add a forgotten file to a running feature (F47). Escape in the
attachment-preview / replay / publish dialogs also navigates back (F35).

## J7 — Deciding a gate

**Entry:** `gate_required` event (auto-navigates from anywhere, replace-mode so Back
isn't polluted) or the "Decide Gate" button on an awaiting-gate step card.

1. `GateView` modal shows pipeline context, the gate artifact, and actions.
2. Approve / Redirect (feedback textarea) / Abort feature. A blocked banner disables
   Approve/Redirect while an earlier step is still active; Abort stays enabled.
3. Decision → `gate_decide` → back to Feature Detail.

**Friction:** the spec says "full-screen takeover"; the implementation is a max-w-2xl
modal — fine, but the spec should be updated (F28). Gate accent color is amber here and
in the timeline, violet in the Project Home list (F27). The artifact pane inside the
gate is Monaco-backed, so gate review is broken offline (F34).

## J8 — Sync & merge-conflict resolution

**Entry:** "Sync with main" on a terminal-state feature.

1. `feature_sync` merges `origin/<default>` into the feature branch.
2. Clean → green banner. Conflict → file list + "Resolve with agent"
   (`feature_resolve_sync_conflicts` spawns a resolver agent, merges back, optionally
   replays validation) or manual resolution via the agent terminal.

**Friction:** backend emits `conflict_detected` but no frontend listener exists — the
only way to learn about a conflict is to have triggered the sync yourself (F24-adjacent,
see F33).

## J9 — Publishing and lifecycle

**Entry:** Feature Detail terminal-state actions.

1. Publish MR → title prompt (pre-filled from description) → `publish_mr` (idempotent)
   → MR badge row with provider-state refresh; `mr_merged` event fires a toast +
   notification.
2. Cleanup → applies the project `feature_lifecycle` (archive / keep / auto-delete with
   force-delete fallback prompt).

## J10 — Remote runs

> **Rewritten July 2026.** This journey used to run through a dedicated Runs tab (the
> "return inbox"). That tab was removed as redundant with Project Home; see
> `docs/REMOTE_EXECUTION_PLAN.md` M6.2 amendment for the removal and the tradeoffs
> accepted with it.

**Entry:** Project Home (remote/detached pipeline cards) → the run's feature. There is no
longer a global Runs destination, TopBar button, or command-palette entry.

- Project Home lists the open project's active features, remote ones included; opening one
  lands in FeatureDetail, which owns the run.
- FeatureDetail carries the per-run affordances: the event-log tail (`RunEventTimeline`),
  inline gate Approve/Reject (`RemoteGateActions`), credential re-injection
  (`ReinjectCredentials`), Cancel for a running/parked run (`CancelRunButton`), and a
  branch-diff link for a pushed branch with no PR (`DiffLinkButton`). PR-ready runs link
  out to the PR.
- The bucket taxonomy (Parked / Needs credentials / Failed / PR ready / Running /
  Unreachable / Cancelled) survives as `bucketFor` in `lib/runStatus.ts` and still drives
  which actions render.
- Startup reconciliation (`remote_reconcile_runs`) fires desktop notifications for newly
  actionable runs.

**Friction:** Cancel still has no confirmation (F23). Attention is now *pull*, not push —
with the TopBar badge gone, a parked or failed run's only passive signal is the
startup-reconcile desktop notification, and a run in a project you don't have open (or on
an archived feature) is listed nowhere. Accepted knowingly; recorded in the M6.2 amendment.

## J11 — Workflow authoring

**Entry:** TopBar "Workflows", command palette.

- `WorkflowList`: starter vs custom groups, schedule badges, preview pane with step
  timeline; actions: Run (▶ opens the start-feature modal pre-pinned), Edit, Export,
  Revert-to-default (starters) / Delete (custom).
- `WorkflowEditor`: form-first builder; every save creates a new `WorkflowVersion`.

**Friction:** ▶ Run silently no-ops with no active project (F15). Delete/revert use raw
`window.confirm` while the rest of the app uses styled dialogs (F23). There is no Import
button in the UI even though `workflow_import` is a registered command (F12-adjacent).
The editor discards unsaved edits with zero warning on Back/Escape/mouse-back (F38);
deleting a step leaves dangling `on_failure` loopback references, and new steps silently
pre-pin `opencode` instead of the project default (F39). No version-history viewer
exists despite the versioning model (F39).

## J12 — Settings, preferences & help

**Entry:** gear icon, `Cmd/Ctrl+,`, command palette; per-project gear → Workspace
Settings (General & Repositories / Agent Strategy / Workflow Overrides / Project Memory,
with re-bootstrap + dirty-repo data-loss warnings + delete-workspace flow).

- Global Preferences: Machines (SSH hosts, secrets, runner install), Providers (link),
  Defaults (workspace dir, agent timeouts), Memory agent, About.
- Help: F1 / `?` / palette → `DocsPanel` (7 bundled markdown pages).

**Friction:** the docs panel fetches dev-server paths and cannot load any page in a
production build; its markdown renderer skips tables, so the Keyboard Shortcuts page is
empty even in dev (F4). The purpose-built `ShortcutHelp` overlay is never mounted (F5).
The About tab lists data paths that don't match where data actually lives (F11).
Workspace-dir save failures are silently swallowed (F29), as are memory-agent save
failures (F45). Opening the Machines tab SSH-probes every configured machine (F43);
"Test Connection" in the machine modal silently saves the machine first (F37); machine
deletion never checks for dependent projects (F43). Within Workspace Settings, the
Overrides tab saves instantly while other tabs need the Save button — two save models in
one screen (F49).
