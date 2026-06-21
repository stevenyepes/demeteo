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

2. **Sub-task worktrees**: Each sub-task within a parallel step gets its own Git worktree at `demeteo/<subtask-id>`. Worktrees allow concurrent sub-tasks without branch switching conflicts.

3. **Sequential merge**: After all sub-tasks in a parallel step complete, their worktrees are merged sequentially into the feature branch in the order they were defined.

4. **Conflict detection**: If a merge produces conflicts, Demeteo records the conflict details and triggers the configured resolution policy (gate for manual resolution, or auto-agent cascade).

## Publishing

Once the feature completes, you can publish it as a Pull Request (GitHub) or Merge Request (GitLab) from the Feature Detail screen. The MR/PR includes the full feature branch history.

## Lifecycle

After a feature is completed and its MR is merged, you can apply the project's lifecycle policy:

- **Archive**: Soft-deletes the feature record (keeps branches for reference)
- **Auto-delete**: Removes the feature branch and soft-deletes the record
- **Keep**: Preserves everything indefinitely
