# Creating a workspace

A **workspace** in Demeteo is the pairing of a Git repository with a host machine where agents will run. Workspaces appear in the left rail and are the unit you start features against.

![The sidebar's workspaces list, with the + button to create a new workspace](../assets/screenshots/workspace-sidebar.png)

## How to create one

Click the **`+`** button next to the *Workspaces* header in the left rail (or the wand icon to its right for the guided wizard).

There are two creation paths:

### 1. Connect an existing repository

For repos that already live on GitHub / GitLab, choose a [provider](../settings.md#providers), pick a namespace/group, and Demeteo will:

- Clone the repo into your configured [workspace storage directory](../settings.md#defaults).
- Detect the default branch and any PR template.
- Index the worktree strategy (branch naming, merge flow).
- Show you a **proposed worktree strategy** you can approve or edit.

### 2. Create from zero

For a brand-new repo, the guided wizard walks you through **seven one-decision-per-screen steps**:

1. **Name** — pick a workspace name (used in the sidebar).
2. **Provider** — choose a connected Git provider and namespace.
3. **Group** — personal, organization, or sub-group.
4. **Machine** — *This machine* (local) or a remote SSH host you've [configured](../settings.md#machines).
5. **Agent** — pick the default coding agent (`opencode`, `claude-code`, `hermes`, `codex`).
6. **Model** — pick the model the agent will use.
7. **Description** — confirm visibility and any notes.

On the final step, Demeteo **auto-launches the Standard Feature Pipeline** against the freshly-created repo so you go straight from "no workspace" to "first feature running" in one click.

## After creation

The new workspace appears in the left rail with a status dot:

- **Green (ready)** — repo cloned, worktree strategy approved, ready for features.
- **Amber (syncing)** — initial clone in progress.
- **Ruby (error)** — clone failed; hover for the reason.

Per-workspace settings (agent strategy, conflict policy, defaults) live under **Project Settings** — see [Settings](../settings.md) for the full surface.