# Epic UX2 — Truthful UI & surfaced capability

> **Roadmap source:** [03-roadmap-6-months.md § Epic UX2](../03-roadmap-6-months.md#epic-ux2--truthful-ui--surfaced-capability); rank 10 in [04-high-level-plan.md](../04-high-level-plan.md); ships **v1.2 (Nov)**.

**Outcome:** what the UI shows matches what the backend does — no fabricated telemetry, no silently dropped input, no silently swallowed errors, and no registered backend command without either a UI surface or a recorded decision that it stays headless. Closes the [UX audit](../../ux-audit/findings.md)'s P2 tier.

**Out of scope:** P3 polish (Epic UX3, Later); visual redesign.

**Epic acceptance:** audit P2 list closed, or per-item waived at the M3 review with a decision record; every registered Tauri command has a UI surface or a documented headless-only decision; provider alias persists and edit means update, not re-connect (F8).

**Why F28 (start-feature/strategy-form consolidation) matters beyond this epic:** it's explicit groundwork for Epic C1 — the board reuses these surfaces, so per the roadmap, consolidation must precede the board, not follow it. **Check Story UX2.5's status before Epic C1's Story C1.3 starts.**

**Waiver process:** per the roadmap's decision-checkpoint table, any P2 finding not shipped in v1.2 needs a per-item decision record at the M3 (end-Oct) review — waiving silently is not an option; document it in `docs/DECISIONS.md` or `docs/OPEN_QUESTIONS.md` as appropriate.

**Source of truth for line numbers:** all references below are as of commit `82fa581` per `docs/ux-audit/findings.md`'s header — re-verify against current `HEAD` before editing.

---

## Story UX2.1 — Truthful state & copy (F9, F10, F11, F42, F21, F22, F41)

**As a** user looking at Project Home, **I want** the status chips, header copy, and metrics to reflect reality, **so that** I can trust what the app tells me.

**References:** `docs/ux-audit/findings.md` F9, F10, F11, F42, F21, F22, F41.

**Status:** Not started.

**Tasks:**
- [ ] F9: `get_active` (`crates/demeteo-core/src/adapters/database/repos/feature.rs:52-58`) returns every non-archived feature including `failed`/`completed`/`cancelled`/`awaiting_mr`; `ProjectHome.tsx:969-981` renders all of them as a pulsing "RUNNING FLEET." Mirror `FeatureDetail`'s correct status-chip logic (running / verifying / gated / completed / failed / cancelled) here, and rename the "Active Running Pipelines" section header to match. **Reuse this fixed logic in Epic C1's board cards (Story C1.4) — don't let the board reimplement the same bug.**
- [ ] F10: `ProjectHome.tsx:504` hardcodes "Connected via GitHub Enterprise • Default Workflow: Standard Feature Pipeline" for every project. Replace with the actual provider and default workflow from project settings.
- [ ] F11: `PreferencesScreen.tsx:348-352` claims `~/.demeteo/` for data/logs/artifacts; actual path is `~/.local/share/com.stvcloud.demeteo/` (platform equivalent) per the README/Tauri identifier. Fix the displayed path.
- [ ] F42: `ProjectSettingsContext.tsx:506,531` writes a fabricated `nodes: computeType === 'local' ? 4 : 8` into project state, which then renders as a real metric. Remove the fabricated number; either compute a real measurement or remove the "nodes" metric from the UI entirely.
- [ ] F21: `StartFeatureModal.tsx:68` and `wizard/CreateProjectWizard.tsx:32` include `antigravity` in `AGENT_KINDS`, while `ProjectHome.tsx:739`/`FeatureDetail.tsx:404` filter it out and the README says unsupported. Remove `antigravity` from the two pickers that still offer it — one consistent agent-kind list everywhere, ties directly to Epic A2's de-scope decision record.
- [ ] F22: `App.tsx:440-446` + `EmptyStateCard.tsx` — "Sync Worktrees" actually opens the new-project form, "Deploy Agents" actually opens the workflow list. Rename tiles to match destinations (e.g. "Bootstrap a project", "Browse workflows").
- [ ] F41: `wizard/CreateProjectWizard.tsx:247-259` shows the project in the rail as `draft.title || draft.name` — `draft.title` is the *feature* title, not the project name, so the rail shows the feature title until restart. Fix to always show `draft.name`.

## Story UX2.2 — Silent failures become visible (F29, F45, F37, F38, F13)

**As a** user whose save or connection action fails, **I want** to be told it failed, **so that** I don't believe an action succeeded when it didn't.

**References:** `docs/ux-audit/findings.md` F29, F45, F37, F38, F13.

**Status:** Not started.

**Tasks:**
- [ ] F29: `PreferencesScreen.tsx:79-90`'s `handleSaveWorkspaceDir` has no `catch` — a backend error leaves the spinner reset with the user believing the save happened. Add error handling + visible failure state.
- [ ] F45: `MemoryAgentSettings.tsx:76-91` — failed save only `console.error`s, button just stops spinning with no "Saved"/error indication. `PreferencesScreen.tsx:46-58`'s Defaults-tab load has no catch either, leaving fields silently blank on rejection. Fix both.
- [ ] F37: `EnvModal.tsx:118-139`'s "Test Connection" calls `ensureSaved()` first, which actually creates/saves the machine (writing the secret to the keyring) before testing — a user who tests then cancels has still created a machine. Either make "Test" truly read-only, or relabel the button to state the save side-effect explicitly.
- [x] ~~F38~~ **done 2026-07-26 (P3.3)** — closed by construction when the canvas builder replaced `WorkflowEditor`; `useNavigationGuard` covers Back/Escape/mouse-back. Original: `WorkflowEditor.tsx` has no dirty-state guard — Back arrow (`:204`), Escape/`Cmd+W`, and mouse-back all silently discard unsaved steps/prompt-templates/schedule edits. Add a dirty-state check that warns before discarding (prompt templates especially can't be recreated from memory).
- [ ] F13: `App.tsx:527-555`'s remote-launch path forwards only workflow/title/description/agent/model/loop/unattended/caps — staged attachments, per-step overrides, and the commit-artifacts choice are dropped with no warning. Minimum fix: disable or clearly annotate those controls when a remote machine is selected, so the modal doesn't imply they'll be honored.

## Story UX2.3 — Surface built capability (F12, F24, F44, F47, F26)

**As a** user, **I want** backend capabilities that already exist (pause/resume, workflow import, conflict notifications, post-sync revalidation, post-launch attachments, notification click-through) to actually have a UI, **so that** I don't have to know Rust internals to use features Demeteo already built.

**References:** `docs/ux-audit/findings.md` F12, F24, F44, F47, F26.

**Status:** Not started.

**Tasks:**
- [ ] F12: `feature_pause`/`feature_resume` are registered commands (`src-tauri/src/lib.rs:373-374`) with zero UI. `docs/UX_JOURNEYS.md` J6 already lists Pause/Resume as actions — add the buttons to `FeatureDetail`. Also add a workflow Import button to `WorkflowList` (paired with the existing Export button; `workflow_import` is registered but has no UI affordance).
- [ ] F24: backend emits `conflict_detected` (`adapters/tauri_ui/notification.rs:42-45`) with no frontend listener. Wire it to a toast/notification — `NotificationBell` already has a `merge_conflict` kind and accent ready (`NotificationBell.tsx:231,248`), so this is a listener + dispatch, not new UI.
- [ ] F44: the "resolved" banner tells the user to manually confirm the build still works. The `revalidateStepExecutionId` parameter the finding names was removed — no caller ever passed one, so the replay it fed was unreachable. Re-validating after a merge needs the step id resolved on the Rust side, not a parameter added back to the command.
- [ ] F47: `feature_add_attachment`/`feature_remove_attachment` are registered, and `AttachmentDropzone` already implements a `mode="direct"` path that persists immediately — but nothing renders `direct` mode, and `FeatureDetail`'s attachment chips are read-only. Render the existing direct-mode dropzone in `FeatureDetail` so users can attach files to a running feature.
- [ ] F26: `NotificationBell.tsx:94-108` — clicking a notification only marks it read (the code comment admits navigation is "Future"). `gate_pending` notifications carry the feature id already; wire click-through navigation to the relevant gate/feature, plus a mark-all-read action.

## Story UX2.4 — Navigation correctness (F14, F16, F7)

**As a** user navigating between views, **I want** the back-stack and project-scoped feature cycling to behave correctly, **so that** Escape/Back and `Cmd+G` don't misbehave.

**References:** `docs/ux-audit/findings.md` F14, F16, F7.

**Status:** Not started.

**Tasks:**
- [ ] F14: `NavigationContext.tsx:20-48`'s `shallowEqualView` has no case for `'remote-inbox'` (falls to `default: return false`), so re-clicking "Remote runs" pushes a duplicate back entry every time. Add the missing case, and add a sane `default` that compares by view kind so any future view kind doesn't silently inherit the same bug.
- [ ] F16: `App.tsx:211-233`'s `feature_status_changed` handler appends any unknown feature id to the `Cmd+G` cycling list keyed to `currentProjectId` without checking which project the event actually belongs to — events from a second project (or a remote mirror) pollute the current project's cycle list with phantom entries. Filter events by project id before appending.
- [ ] F7: `NewProjectView.tsx:225-232`'s `newProj` omits `compute_type`/`remote_host`/`tokens`; `ProjectHome.tsx:514` keys the Pipelines/Terminal tab bar and machine-id resolution off `activeProject.compute_type`, so a freshly-created remote project behaves as local until restart. Carry these fields into the UI state at creation time.

## Story UX2.5 — C1 groundwork: consolidate start-feature and strategy forms (F28)

**As a** maintainer, **I want** the triplicated worktree-strategy form and the two parallel start-feature UIs unified into single components, **so that** Epic C1's board can build on one surface instead of three drifting copies.

**References:** `docs/ux-audit/findings.md` F28. **This story blocks Epic C1's Story C1.3** — check this story is done (or at least the workflow-picker/repo-scoping pieces C1 needs) before starting board UI work.

**Status:** Not started.

**Tasks:**
- [ ] The worktree-strategy proposal form exists three times — `NewProjectView.tsx:598-679`, `ProjectHome.tsx:398-485`, `ProjectSettingsShell.tsx:81-133` — already drifting (ProjectHome's version omits the success screen; defaults differ, per F28's note that `UX_JOURNEYS.md` J9 says conflict-policy default should be `auto_agent` but all three forms actually default to `always_gate`). Extract one shared component; fix the default-policy inconsistency as part of the same change (decide which default is correct — the spec or the implementation — and make the doc and code agree).
- [ ] Unify the two parallel "start a feature" UIs: the Project Home composer (no title field — title = description, no remote option, no per-step overrides) vs `StartFeatureModal` (no smart workflow inference). Pick one canonical implementation with the union of capabilities, or a clearly justified reason to keep two — do not just pick one arbitrarily without checking which capabilities are load-bearing for existing users.
- [ ] Along the way, dedupe the smaller repeated helpers noted in the same finding: `formatTokens` (`FeatureDetail.tsx:109` vs `lib/utils.ts:1`), `fuzzyMatch` (`ProjectRail.tsx:6` vs `CommandPalette.tsx:19`), relative-time helpers (`NotificationBell.tsx:257`, `RemoteRunInbox.tsx:88`) — small, low-risk cleanup that's natural to bundle with this consolidation pass.
- [ ] Provider alias fix (F8, listed in the epic's acceptance criteria though not in the F28 cluster): `ProviderSettings.tsx:20-58` requires an alias that's never sent to `connect_provider_instance`, and `App.tsx:295-298` maps `name: p.kind` on reload so the card shows "github" instead of the alias; Edit mode re-runs *connect* instead of update. Fix: persist the alias, and make Edit call an actual update path instead of re-connect with a forced PAT re-entry.
