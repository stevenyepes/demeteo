# Conflict Resolution

When multiple sub-tasks modify overlapping code regions, Git merge conflicts can occur. Demeteo offers a configurable conflict resolution cascade.

## Resolution Policies

Configured at the project level under **Workspace Settings → Agent Strategy & Policies**:

### Always Gate (default)
All conflicts are sent to a gate for manual resolution. The user reviews each conflict file and provides resolution instructions.

### Auto Agent First
Demeteo attempts to resolve conflicts automatically using an agent. The agent receives the conflicting files with `<<<<<<<` / `=======` / `>>>>>>>` markers and tries to produce a clean merge. If the agent fails or the result is unsatisfactory, the conflict cascades to a manual gate.

### Immediate Manual Merge
Skips the auto-agent step and immediately presents conflicts for manual resolution through the conflict viewer.

## The Conflict Viewer

When a gate is triggered for conflict resolution:

1. Each conflicting file is listed with its path and conflict count
2. Click a file to see the merge diff with conflict markers
3. Provide resolution instructions or approve a proposed resolution
4. Submit the gate decision to continue the pipeline

## Preventing Conflicts

- Use well-defined module boundaries
- Configure branch prefix conventions to avoid overlapping changes
- Run sub-tasks that target different files or concerns
