# Findings — bugs, broken UX, inconsistencies, opportunities

Severity scale: **P1** = user-visible breakage or data-loss risk on a core journey;
**P2** = broken or misleading UX on a common path; **P3** = inconsistency / polish /
tech-debt with UX impact. Line numbers are as of commit `82fa581`.

Findings F1–F33 are from the first pass (main views + libs). F34–F49 are from the
second pass (WorkflowEditor, MachinesView/EnvModal, terminal components,
CodeEditorView/ArtifactViewer, settings tabs, wizards, remaining libs/hooks).

---

## P1 — High severity

### F1. "Stop Step" cancels the entire feature
`FeatureDetail.tsx:437-452` — `handleStopStep` invokes `feature_cancel`, which the
backend maps to `executor.feature_cancel` (`src-tauri/src/commands/features.rs:152`),
cancelling the whole feature. The button says "Stop this step execution" and the confirm
dialog says "stop the execution of this step". A user trying to nudge one stuck step
kills their entire pipeline. Either rename the button/dialog to "Cancel Feature" or add a
real per-step stop command.

### F2. Repository targeting is collected in three places and never sent anywhere
`start_feature` (`src-tauri/src/commands/features.rs:100-114`) has **no repo parameter**.
Yet:
- `ProjectHome.tsx:41,626-657,688-717` — suggested-repo chips and Customize checkboxes
  maintain `selectedRepos`; `handleStartFeature` (`:279`) never sends them.
- `StartFeatureModal.tsx:310` — computes `targetRepos` and passes it to `onLaunch`;
  `App.tsx:523-615` ignores the field entirely.
- `StartFeatureModal.tsx:456` — copy says "(auto-detected from description; edit in
  Customize)" but the Customize section contains no repo editor at all.

Users believe they are scoping the run; the orchestrator always uses its own selection.
Either wire `target_repos` through `start_feature` or remove the pickers and label the
chips as informational.

### F3. Launch-modal conflict detection marks every repo as conflicted
`StartFeatureModal.tsx:273-302` — for each active feature it calls
`get_repositories_for_project(f.project_id)`, i.e. **the same project's full repo list**,
so `usedRepos` contains every project repo whenever *any* feature is active. The guard
`if (f.id !== /* self */ undefined)` compares an id to `undefined` and is always true.
Result: the amber "conflict" badge is noise, training users to ignore a warning that was
supposed to prevent parallel-run collisions. Needs a real per-feature repo association
(which also depends on F2).

### F4. In-app documentation is unreadable in production; shortcuts page empty everywhere
`DocsPanel.tsx:34-52` fetches `/src/docs/${slug}.md` (a Vite dev-server path) with a
fallback to `/docs/${slug}.md` — but `public/` contains only icons, and the `.md` files
are not bundled. In a packaged build every page shows "Document not found" (or the SPA
fallback HTML). Additionally `SimpleMarkdown` (`DocsPanel.tsx:169-170`) skips all `|`
table lines, and `src/docs/keyboard-shortcuts.md` is one big table — so the Keyboard
Shortcuts page renders with no content even in dev. Fix: import the docs with Vite's
`?raw` glob imports (bundled at build time) and render with `react-markdown` +
`remark-gfm`, which are **already runtime dependencies** used by `ArtifactViewer`.

### F5. Keyboard-shortcut system: three sources of truth, all disagreeing; help overlay is dead code
- `lib/shortcuts.ts` (registry rendered by `ShortcutHelp`) documents `Cmd+P` (palette
  alias), `Cmd+R` "Reload data", and `F11` fullscreen — **none are implemented** in
  `hooks/useKeyboardShortcuts.ts`. Worse, unhandled `Cmd+R` falls through to the webview
  and reloads the app, dropping all UI state.
- The dispatcher implements `Cmd+Shift+F` and `Cmd+.` for the palette
  (`useKeyboardShortcuts.ts:72-77,97-99`) — absent from the registry.
- The `?` help chord is dead on standard layouts: `useKeyboardShortcuts.ts:37` requires
  `!e.shiftKey`, but typing `?` requires Shift on US/most layouts, so the handler never
  fires. (`Cmd+?` at `:101-104` works but is documented nowhere in the registry.)
- `ShortcutHelp.tsx` + `ShortcutsContext.tsx` (781 lines incl. tests) are **never
  mounted** — not exported from `context/index.tsx`, not imported by any view. F1/`?`
  and the palette's "Keyboard Shortcuts" entry all open the generic `DocsPanel` instead
  (whose shortcuts page is empty, F4).

Fix: make `useKeyboardShortcuts` consume the registry's matcher (`matchesEntryKeyboard`
already exists for exactly this), mount `ShortcutHelp` on F1/`?`, and delete or implement
the phantom entries.

### F6. Retrying a failed bootstrap duplicates project records
`NewProjectView.tsx:153-208` — `handleCreate` always calls `create_project` first; the
error screen's "Retry Build" (`:291`) re-invokes `handleCreate`, creating a **second
project row** (and a third on the next retry). Also, cancelling at the strategy-proposal
step leaves the already-created project invisible until the next app restart (it was
never added to UI state). Retry should reuse the `projectId` already captured at `:189`
and only re-run `bootstrap_project`.

---

## P2 — Broken or misleading UX

### F7. Newly created project loses `compute_type` / `remote_host` in UI state
`NewProjectView.tsx:225-232` — `newProj` omits `compute_type`, `remote_host`, and
`tokens`. `ProjectHome.tsx:514` keys the Pipelines/Terminal tab bar and machine-id
resolution off `activeProject.compute_type`, so a remote project behaves as local until
the app is restarted.

### F8. Provider alias is fake; "Edit" silently re-creates
`ProviderSettings.tsx:20-58` requires a "Provider Name / Alias" but
`connect_provider_instance` is called with only `providerType`/`host`/`pat` — the alias
is never persisted, and `App.tsx:295-298` maps `name: p.kind` on reload, so the card
shows "github" instead of the user's alias. Edit mode passes no `id` and re-runs
*connect* (with a forced PAT re-entry and no explanation), relying on backend dedupe
semantics the UI doesn't communicate.

### F9. Project Home pipeline list renders failed/completed features as "RUNNING FLEET"
`get_active` (`crates/demeteo-core/src/adapters/database/repos/feature.rs:52-58`) returns
every feature not archived/deleted — including `failed`, `completed`, `cancelled`,
`awaiting_mr`. `ProjectHome.tsx:969-981` renders only two chip states: `gated` or a
pulsing cyan "RUNNING FLEET". A failed pipeline looks alive; a completed one never reads
as done. The section header "Active Running Pipelines" is equally wrong. Mirror the
status-chip logic from `FeatureDetail` (running / verifying / gated / completed /
failed / cancelled) here.

### F10. Hardcoded header copy lies about provider and workflow
`ProjectHome.tsx:504` — every project says "Connected via GitHub Enterprise • Default
Workflow: Standard Feature Pipeline" regardless of the actual provider (possibly GitLab,
possibly none) and the actual default workflow.

### F11. About tab lists wrong data paths
`PreferencesScreen.tsx:348-352` claims `~/.demeteo/` for data/logs/artifacts. The README
and Tauri identifier put the database at
`~/.local/share/com.stvcloud.demeteo/` (platform equivalent). Users following the About
panel to find their data will look in a directory that doesn't exist.

### F12. Backend capabilities with no UI: pause/resume, workflow import
`feature_pause` / `feature_resume` are registered commands
(`src-tauri/src/lib.rs:373-374`) and `docs/UX_JOURNEYS.md` J6 lists Pause/Resume as
actions — no button anywhere invokes them. Likewise `workflow_import` is registered and
`workflow_export` has a UI button, but there is no Import affordance in `WorkflowList`.

### F13. Remote launches silently drop attachments, per-step overrides, and commit-artifacts
`App.tsx:527-555` — the `remote_submit_run` path forwards only workflow/title/description
/agent/model/loop/unattended/caps. Staged attachments (admitted in a comment), per-step
overrides, and the commit-artifacts choice are dropped with **no warning in the modal**.
A user who staged screenshots and picked a remote machine gets an agent that never saw
them. Minimum fix: disable/annotate those controls when a machine is selected.

### F14. Back-stack pollution for view kinds missing from the equality check
**Partly resolved by deletion (July 2026).** The `'remote-inbox'` view no longer exists
(the Runs tab was removed — see `REMOTE_EXECUTION_PLAN.md` M6.2 amendment), so the
specific duplicate-back-entry bug is gone with it.

The underlying defect is *not* fixed: `NavigationContext.tsx:20-48`'s `shallowEqualView`
still falls to `default: return false` for any view kind it has no case for, so the next
view kind added will silently inherit the same bug. The remaining work is the `default`
that compares by view kind.

### F15. Workflow ▶ Run silently no-ops with no active project
`App.tsx:514` renders `StartFeatureModal` only when `currentProjectId && currentProject`.
`WorkflowList`'s Run button dispatches `OPEN_START_FEATURE` unconditionally
(`App.tsx:476-486`) — with zero projects (reachable from the empty state via "Deploy
Agents") nothing happens, no toast, no hint. Same for the `Cmd+T` no-project case, which
at least guards but is equally silent.

### F16. Cmd+G cycling list can mix in other projects' features
`App.tsx:211-233` — the `feature_status_changed` handler appends any unknown feature id
to the cycling list with `project_id: currentProjectId` and title `'Feature'`, without
checking which project the event belongs to. Events from a second project's run (or a
remote mirror) pollute the current project's `Cmd+G` cycle with phantom "Feature"
entries.

### F17. `FeatureDetail` violates the Rules of Hooks
`FeatureDetail.tsx:165` returns `null` when `view.kind !== 'detail'` **before** ~30 hook
calls. It only works because `App.tsx` unmounts it on view change; any refactor that
renders it in another state crashes with a hooks-order error. Move the guard into the
parent or below the hooks.

### F18. `.worktree-ref.json` artifact classification is unreachable
`FeatureDetail.tsx:61-66` — the `.json` check precedes the `.worktree-ref.json` check,
so worktree refs always classify as plain JSON (wrong icon, wrong "JSON" label, wrong
color). Reorder the checks.

### F19. `step_progress` clobbers the pipeline-total cost
`FeatureDetail.tsx:304-313` — the handler does `setTotalCost(payload.cost_usd)`, i.e.
sets the **pipeline total** to one step's running cost; the header chip visibly drops to
the step cost until `loadFeatureData` recomputes it. It also triggers a full reload
(steps + feature + models probe) on every progress tick.

### F20. Vision warning false-positives when the model field is blank
`StartFeatureModal.tsx:261-269` checks `modelSupportsImagesByName(agentKind, model)`
against the *typed override only*; blank means "inherit project default", but
`modelImageSupport.ts:33` returns `false` for empty strings — so attaching an image with
no override always warns "Model (unset) does not read images" even when the project
default is Claude/GPT-4/Gemini. `ProjectHome.tsx:836-866` gets this right by falling back
to `defaultModel`. Also both warning banners mix a violet border with a ruby background
(`border-violet-500/40 bg-ruby-500/10`) — looks like a palette typo.

### F21. `antigravity` offered in some pickers, banned in others
`StartFeatureModal.tsx:68` and `wizard/CreateProjectWizard.tsx:32` include `antigravity`
in `AGENT_KINDS`, while `ProjectHome.tsx:739` and `FeatureDetail.tsx:404` filter it out
and the README declares it "not currently supported". Picking it in the launch modal or
wizard sets users up for a mid-pipeline failure.

### F22. Empty-state tile labels don't match destinations
`App.tsx:440-446` + `EmptyStateCard.tsx` — "Sync Worktrees" navigates to the new-project
form; "Deploy Agents" opens the workflow list. Neither syncs worktrees nor deploys
anything. Rename to what they do ("Bootstrap a project", "Browse workflows").

### F23. Destructive-action confirmation is inconsistent (and sometimes absent)
- `WorkflowList.tsx:43,54` uses raw `window.confirm` (unstyled native dialog).
- `FeatureDetail` uses Tauri `confirm`/`message` dialogs.
- `ProvidersPage`/`ProjectSettings` use custom styled modals.
- No confirmation at all: deleting a provider with no dependent projects
  (`ProvidersPage.tsx:26-35`), cancelling a remote run (`CancelRunButton`,
  `RunEventTimeline.tsx:334-345` — still unconfirmed after the Runs tab was removed and
  the button moved into `FeatureDetail`).

### F24. Backend emits `conflict_detected`; nothing listens
`adapters/tauri_ui/notification.rs:42-45` emits it; no `useTauriEvent`/`listen` consumer
exists in `src/`. Conflicts surface only if the user manually triggered the sync. Wire it
to a toast/notification (the NotificationBell already has a `merge_conflict` kind and
accent ready at `NotificationBell.tsx:231,248`).

---

## P3 — Inconsistencies, polish, tech debt

### F25. Command palette over-promises
`CommandPalette` placeholder and TopBar hint say "Search projects, features, workflows,
settings…" (`CommandPalette.tsx:98`) but entries are only projects + 7 static nav actions
(`App.tsx:404-420`). Features and workflows are not searchable. The "Keyboard Shortcuts"
entry is a duplicate of "Documentation" (both just open the docs panel).

### F26. Notifications are dead ends
`NotificationBell.tsx:94-108` — clicking a notification only marks it read; the comment
admits navigation is "Future". `gate_pending` / `step_failed` notifications that beg for
a click-through go nowhere. No mark-all-read either.

### F27. Status color language drifts across views
`UX_JOURNEYS.md` §2 defines Emerald=Running. In practice: FeatureDetail header running =
emerald, timeline running = cyan, ProjectHome running = cyan; gated = amber in
FeatureDetail/GateView but violet in ProjectHome (`ProjectHome.tsx:969-976`); completed =
cyan in the header chip. Pick one mapping (a shared `statusColor()` helper) — `StatusBadge`
already exists in `ui/`.

### F28. Triplicated forms and parallel implementations drifting
- The worktree-strategy proposal form exists three times: `NewProjectView.tsx:598-679`,
  `ProjectHome.tsx:398-485`, `ProjectSettingsShell.tsx:81-133` — already drifting
  (ProjectHome's version omits the success screen; defaults differ). Extract one
  component.
- Two parallel "start a feature" UIs (ProjectHome composer vs `StartFeatureModal`) with
  different capabilities (composer: no title field — title = description, no remote,
  no per-step overrides; modal: no smart workflow inference).
- `formatTokens` duplicated (`FeatureDetail.tsx:109` vs `lib/utils.ts:1`); `fuzzyMatch`
  duplicated (`ProjectRail.tsx:6` vs `CommandPalette.tsx:19`). The relative-time helper
  was also duplicated in `RemoteRunInbox.tsx`; that copy died with the Runs tab, leaving
  only `NotificationBell.tsx:257` — no dedupe needed there any more.
- Spec/UI default mismatch: `UX_JOURNEYS.md` J9 says conflict policy default is
  `auto_agent`; all three strategy forms default to `always_gate`.

### F29. Silent failure paths
- `PreferencesScreen.tsx:79-90` — `handleSaveWorkspaceDir` has no `catch`; a backend
  error leaves the spinner reset and the user believing the save happened.
- `FeatureDetail.tsx:610-625` — the MR-state effect refetches `feature_get` on every
  derived-status change (duplicate of the fetch inside `loadFeatureData`) and pipes
  failures to an "internal" toast — noisy on flaky IPC, silent about which call failed.

### F30. Escape-handling contract not honored by modals
`App.tsx:113-118` documents that per-modal ESC handlers "are expected to call
`event.stopPropagation()`". `StartFeatureModal.tsx:346-352`'s `onKey` does not — ESC with
focus inside the modal fires both the local close and the global `pickEscapeAction`
(currently harmless because both close the same modal; a landmine for reordering).

### F31. Remote-inbox live log copy contradicts behavior for terminal runs
**Resolved by deletion (July 2026).** The banner lived in `RemoteRunInbox.tsx:335-339`;
the Runs tab and that component are gone (see `REMOTE_EXECUTION_PLAN.md` M6.2 amendment).
The surviving log viewer, `RunEventTimeline.tsx:479-484`, already gets this right: it
branches on `isTerminal`, so the "Still retrying every 2s" copy only shows for a live run
(where it is true) and a terminal run gets "Couldn't fetch the log from `<machine>`".

### F32. Collapsed rail hides projects beyond the first 8
`ProjectRail.tsx:68` slices to 8 with no overflow affordance; `Cmd+1..9` and the expanded
rail address 9+.

### F33. Misc smaller items
- `App.tsx:243` — the ErrorToast "open-feature" CTA re-navigates to the *current* view
  (no-op unless already on a detail view); it can't actually take you *to* the feature.
- `ProjectHome.tsx:76,1073` — remote model probing uses `remote_host` as a machine id
  with `|| 'local'` fallback; a remote project with an unset host silently probes the
  local machine.
- `StartFeatureModal.tsx:167-194` — the open/reset effect lists `workflowId` in its
  dependencies and doesn't reset `workflowId` on close; reopening for a different
  workflow can briefly show the stale selection.
- `StartFeatureModal.tsx:324-326` — cost/wall-clock caps are sent whenever a machine is
  selected, even after the user toggles Unattended back off (fields are hidden but
  values retained).
- Loop-iterations input advertises 1–10 via HTML attrs but any typed integer (0, 99) is
  passed through unvalidated (`StartFeatureModal.tsx:702-710`).
- `TerminalWindow`/`AgentTerminalDrawer` receive `projectId={featureId}` in
  `FeatureDetail.tsx:1442` — works only because the prop is used as an opaque session
  key; rename the prop (`sessionKey`) to stop the type lie.
- `docs/UX_JOURNEYS.md` J3 calls `NewProjectView` a "slim modal"; it's a full two-column
  view. Update the spec.
- Dead code: `ShortcutsContext.tsx` + `ShortcutHelp.tsx` (see F5),
  `lib/features.ts:listBlockingPredecessor` (unused), the M6.1 `remoteLaunchInfo` banner
  in `App.tsx:619-659` is documented as "superseded by the M6.2 return inbox" but still
  ships as a modal that blocks the whole window for a non-blocking confirmation.

---

# Second pass (F34–F49)

## P1 — High severity

### F34. Monaco is CDN-loaded at runtime — code and artifact views need internet
`@monaco-editor/react` is used by `CodeEditorView.tsx:3-4` (Browse Code),
`ArtifactViewer.tsx:4` (artifact panes in Feature Detail **and** GateView), but the
`monaco-editor` package is not a dependency and no `loader.config` exists anywhere — the
library's default loader fetches Monaco from the jsdelivr CDN at runtime. On an offline
or firewalled machine, "Browse Code", every artifact preview, and the gate-approval
artifact pane show an infinite loading state. For a desktop app whose local pipeline
otherwise works offline, this silently breaks the review half of the core journey (and
phones out to a third-party CDN). Fix: add `monaco-editor` and call
`loader.config({ monaco })` once, so Vite bundles it.

### F35. Escape is double-handled by every dialog — closing a modal also navigates back
`App.tsx:113-118` documents that per-modal ESC handlers must call
`event.stopPropagation()`. **None do**, and the shared primitives can't:
- `ui/Modal.tsx:11-16` and `PromptDialog.tsx:39-46` add their own window-level Escape
  listeners without stopping propagation.
- The global hook (`useKeyboardShortcuts.ts:26-29`) fires on the same keypress; none of
  these dialogs are tracked in `UIState`, so `pickEscapeAction` falls through to
  `navigate-back`.

Concretely: pressing Escape to close the attachment preview, the replay-from-step
modal, the Publish-MR prompt, the repo-selection modal, or the dirty-repo warning *also*
pops the navigation stack — the user closes a preview and finds themselves on a
different screen. `AgentTerminalDrawer` has no Escape handling at all, so Escape while
the terminal drawer is open navigates the view *underneath* the drawer. Fix: register
overlays in a single Escape-priority stack (extend `pickEscapeAction`) or have `Modal`
capture Escape in the capture phase and stop propagation. Supersedes the milder F30.

### F36. ~3,200 lines of dead parallel UI implementations ship in the bundle
Nothing in the live app imports any of these (only their own tests do):
- **A complete second create-from-zero wizard**: `CreateFromZeroWizard.tsx` (328) plus
  fourteen `ui/CreateZero*.tsx` step/panel/hook files, `hooks/useCreateProjectWizard.ts`
  (253) and `lib/createProject.ts` — the live wizard is `wizard/CreateProjectWizard.tsx`.
- `CommandSelector.tsx` (241) and `TerminalStatusOverlay.tsx` — unused overlays.
- `ShortcutHelp.tsx` (381) + `ShortcutsContext.tsx` (119) — see F5.

Beyond bundle weight, this is a correctness trap: the two wizards implement the same
seven-step flow with different validation, and a fix applied to the dead one looks done
while changing nothing. Delete or wire them up.

## P2 — Broken or misleading UX

### F37. EnvModal "Test Connection" silently creates/saves the machine
`EnvModal.tsx:118-139` — `handleTestConnection` calls `ensureSaved()` first, which
`add_machine`s / `update_machine`s the record **and writes the secret to the keyring**,
then fires `onSaved` (refreshing the parent list). A user who fills half the form,
clicks "Test", then "Cancel" has still created a machine. A button labeled "Test" must
be read-only, or the save side-effect must be stated on the button.

### F38. WorkflowEditor discards edits with zero warning
`WorkflowEditor.tsx` has no dirty-state guard: the Back arrow (`:204`), the global
Escape/`Cmd+W` (navigate-back), and mouse-back all silently drop every unsaved step,
prompt-template, and schedule edit. Prompt templates are exactly the kind of long-form
text a user cannot re-create from memory.

### F39. WorkflowEditor: dangling `on_failure` targets and other authoring traps
- Deleting or reordering a step (`:100-118`) does not touch other steps' `on_failure`
  loopback pointers — a saved workflow can reference a step id that no longer exists.
- New agent/parallel steps are seeded with `agent_kind: 'opencode'` (`:92`) instead of
  the "Project Default" the selector offers — users inherit a hardcoded harness without
  choosing one.
- `workflow_versions` is a registered command and the spec (J10) promises version
  history round-trips, but the editor shows only "Editing version N" — no history
  viewer, no diff, no revert-to-version (revert-to-*default* exists only for starters).
- Cron validation checks only "5 whitespace-separated fields" (`:145`); no syntax check,
  no next-run preview.

### F40. Mouse back/forward fire underneath open modals despite documented suppression
`lib/shortcuts.ts:264-282` describes XButton1/XButton2 as "Suppressed while any modal is
open or a text field is focused". `useMouseNavigation.ts:27-42` has **no suppression of
any kind** — mouse-back while the Start-Feature modal or terminal drawer is open
navigates the view underneath it (same failure family as F35, but with a registry entry
actively promising the opposite).

### F41. Wizard-created project shows the feature title as its name
`wizard/CreateProjectWizard.tsx:247-259` — on launch the project is added to the rail
with `name: draft.title || draft.name`; `draft.title` is the *feature* title from the
Description step, so the rail shows e.g. "Add OAuth login" as the project name until the
next restart re-reads the DB.

### F42. Fabricated "nodes" telemetry written into project state
`ProjectSettingsContext.tsx:506,531` — saving settings sets
`nodes: computeType === 'local' ? 4 : 8`, a made-up number that then renders as a real
metric in the project rail. Companion to the hardcoded header copy (F10): telemetry that
isn't measured shouldn't be displayed.

### F43. Machines screen SSH-probes every machine on mount; delete has no dependency check
`MachinesView.tsx:193-206` — `fetchMachines` fires `probeRunnerSilent` (an SSH
round-trip) for *every* configured machine each time the Machines tab opens, while the
comment at `:77-79` claims the mount path avoids "auto-probing any remote host over
SSH". Each unreachable machine paints a red error row on every Settings visit. Deleting
a machine (`:287-297`) uses `window.confirm` and — unlike provider deletion, which lists
affected projects — never checks whether projects or remote runs reference the machine.

### F44. Post-sync re-validation exists in the API but is never triggered from the UI
`syncFeature`/`resolveSyncConflicts` accept `revalidateStepExecutionId` to replay the
workflow's validation step after a merge; `FeatureDetail.tsx:552,863` always passes
`null`. The "resolved" banner then instructs the user to "run the validation step to
confirm everything still builds" — a manual step the code was designed to automate
(F12/F47 family: built capability, no UI).

### F45. More silent-failure save paths
- `MemoryAgentSettings.tsx:76-91` — a failed save only `console.error`s; the button just
  stops spinning with no "Saved" and no error.
- `PreferencesScreen.tsx:46-58` — the Defaults-tab load (`get_workspace_dir` +
  `get_workspace_dir_setting`) has no catch; a rejection leaves the fields silently
  blank. Same family as F29.

### F46. Artifact viewing is hardwired to the local machine; worktree-ref CTA is unreachable
`ArtifactViewer.tsx:46-49` reads every artifact via
`sftp_read_file({ machineId: 'local' })` — worth verifying against remote-compute
projects, where step artifacts may only exist on the remote host. The worktree-ref
"Open in Editor" CTA (`:52-67`) only activates when a caller passes
`mime === 'application/x-demeteo-worktree-ref'` — no caller ever passes `mime`, and the
extension-based classifier is also unreachable (F18), so this whole flow is dead in
practice. Also its custom `li` renderer (`:245-250`) draws cyan dot bullets inside
*ordered* lists, producing double markers.

### F47. Attachments are launch-only: no add/remove after the feature exists
`feature_add_attachment` / `feature_remove_attachment` are registered commands and
`AttachmentDropzone` implements a full `mode="direct"` path that persists immediately —
but no component ever renders `direct` mode, and `FeatureDetail`'s attachment chips are
read-only. A user who forgot a screenshot at launch has no way to attach it to a
running feature (e.g. before a gate redirect), despite the backend supporting it.

## P3 — Inconsistencies, polish, tech debt

### F48. CodeEditorView papercuts
- `openDiff` (`CodeEditorView.tsx:199-231`) renders the *error string as file content*
  (`setDiffModified(String(err))`) instead of an error state.
- The 3-second auto-refresh of the open file (`:242-245`) runs forever, including for
  terminal features whose worktree can no longer change.
- Renamed files (`R` status) are diffed by the same path on both refs, which typically
  fails on the base ref.
- The reconnect-relevant effect (`:215`) omits `computeType`/`remoteHost`/`workDir` from
  its deps, so a machine change without a repo change won't reconnect
  (`TerminalWindow.tsx:215` shares the pattern).

### F49. Workspace-settings form inconsistencies
- Mixed save models in one screen: the Overrides tab saves every row instantly ("Changes
  save instantly") while General/Strategy require the global "Save Changes" button —
  users can't build one mental model of when edits persist.
- StrategyTab's harness editor (`StrategyTab.tsx:56-67`) reads inputs via
  `document.getElementById` with static ids and silently overwrites a harness with the
  same name; harnesses can't be edited, only deleted and re-added.
- EnvModal shows the "Ensure public key authentication is configured" warning for all
  auth methods including password (`EnvModal.tsx:358-363`), and `parseConnection`
  (`:95-116`) breaks on IPv6 hosts (`lastIndexOf(':')`).
- Deleting a project-memory entry (`ProjectSettingsContext.tsx:431-434`) has no
  confirmation (F23 family).

---

## Opportunities (not defects)

1. **Search features/workflows from the palette** — the data is one `invoke` away and the
   palette already fuzzy-matches; this would make `Cmd+K` the promised universal switcher.
2. **Persist UI state** — sidebar collapsed, last active project, last Preferences tab
   are all reset on restart; `get_app_session`/`set_app_session` commands already exist.
3. **Global default agent/model** — the Defaults tab explicitly apologizes ("available in
   a future release"); the settings plumbing (project settings merge) already supports a
   fallback layer.
4. **Notification click-through** (F26) is cheap: `gate_pending` carries the feature id;
   navigation from the bell would close the loop on the app's core "intervene only when
   needed" promise.
5. **Feature list cost column** — Project Home shows duration/tokens; cost is the number
   users actually watch (it's already summed per feature in the DB).
6. **Single strategy-form component** (F28) would make the always_gate/auto_agent default
   decision a one-line change instead of three.
7. **Expose `feature_pause`/`feature_resume`** (F12) — pausing a runaway pipeline is
   gentler than the current cancel-only option, and the backend is already there.
8. **Adopt `react-markdown` in DocsPanel** (F4) — deletes ~90 lines of hand-rolled
   renderer and fixes tables, links, and nested lists in one move.
9. **Use the probed vision capability** — `lib/agentModels.ts:modelSupportsImages`
   already resolves `supports_images` from the live model probe, and its doc comment
   says it exists for the Start-Feature modal; the modal instead uses the pessimistic
   name heuristic. Switching fixes the false-positive warning (F20) properly.
10. **Bundle Monaco** (F34) — one `loader.config` call plus a `monaco-editor` dependency
    makes Browse Code, artifact previews, and gate review work offline.
11. **A single overlay/Escape registry** (F35, F40) — one ordered stack that Escape,
    mouse-back, and `Cmd+W` all consult would fix an entire class of "closing X changed
    Y" bugs and make the shortcut registry's suppression claims true.
12. **Expose post-launch attachments** (F47) — render the existing `direct`-mode
    dropzone in Feature Detail; the backend commands are already registered.
13. **Delete the dead implementations** (F36) — or, for `ShortcutHelp`, mount it; it is
    the only component that renders the shortcut registry the help chords promise.
