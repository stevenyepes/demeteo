# Feature Branch Model

Demeteo uses a structured branch model to isolate work and enable safe concurrent development.

## Branch Structure

```
main                          # Default branch (configurable)
├── feature/my-feature        # Feature branch (created per feature)
│   ├── demeteo/sub-1         # Worktree for sub-task 1
│   └── demeteo/sub-2         # Worktree for sub-task 2
```

## How It Works

1. **Feature creation**: When you start a feature, Demeteo creates a branch from the default branch named `feature/<slug>`.

2. **Step worktrees**: Each step runs in its own Git worktree, named for the feature *and* the step (`<repo>_wt_<feature-id>-step-<step-id>`). Including the feature id matters: every feature on a project shares one clone, so a worktree name derived from the step alone would be the same path for two concurrent features — and provisioning a worktree begins by deleting whatever is at that path.

3. **One merge per step**: A step's worktree is merged back into the feature branch once, when the step finishes. A `sequence` step runs each of its tasks in that same single worktree, committing between tasks, so the tasks cannot conflict with one another and there is still only one merge at the end.

4. **Conflict detection**: A step's merge can still conflict if the feature branch moved beneath it (for example, a `sync` step pulled upstream). Demeteo records the conflict details and triggers the configured resolution policy (gate for manual resolution, or auto-agent cascade).

## Publishing

Once the feature completes, you can publish it as a Pull Request (GitHub) or Merge Request (GitLab) from the Feature Detail screen. The MR/PR includes the full feature branch history.

## Lifecycle

After a feature is completed and its MR is merged, you can apply the project's lifecycle policy:

- **Archive**: Soft-deletes the feature record (keeps branches for reference)
- **Auto-delete**: Removes the feature branch and soft-deletes the record
- **Keep**: Preserves everything indefinitely
