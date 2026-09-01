# Conflict Resolution

Demeteo hits merge conflicts in two places, and they are handled differently.

A **step's** task-branch merging back into the feature branch is resolved inline: one
agent turn in the step's own worktree, and if that turn does not clear it, the step
fails. Nothing is offered to you, because the run has not finished and the worktree is
not yours yet.

A **sync** — the feature branch merging `origin/<base>` in — is the one you act on. It
opens a *sync worktree*, a throwaway checkout separate from the feature's, and leaves
the merge sitting there with its markers. The Sync pane is where you finish it.

## The Sync pane

Open it from **Sync** in the feature header. On a conflict it names every unmerged path,
shows what git said, and offers four ways forward.

### Resolve with agent

Spawns an agent in the sync worktree. It clears markers and Demeteo commits the result
— the agent never stages or commits anything itself.

It works in **rounds**. After each turn Demeteo reads every conflicted file and counts
what still has markers; a round that cleared some buys another, over only what is left,
and a round that cleared none stops. So a large conflict finishes in one press, and a
resolver that is not getting anywhere stops instead of spending your budget.

If it does stop early, the reason says what is left and how much — *"2 of 8 files still
have conflict markers: … (1 hunk), … (5 hunks)"*. Pressing **Try again** carries on from
there rather than starting over.

### I've resolved it

For when you would rather do it yourself. Fix the files in the sync worktree, then press
this: Demeteo checks that nothing declared by the merge still carries markers, runs the
project's own checks in the merged tree, and commits and publishes on exactly the terms
the agent path does. No agent is spawned.

If anything is still conflicted it refuses and names it, rather than committing a tree
that is not finished.

### Open a terminal here

A shell in the sync worktree. This is not the feature worktree — the markers are only in
this one, and it is the checkout both of the presses above act on. Clicking a file in the
unmerged list opens that same checkout's copy in the editor.

### Abort sync

Undoes the merge and discards the sync worktree. The branch goes back to where the sync
found it.

## What a fresh Sync will not do

Sync refuses rather than starting over when the existing sync worktree holds work: a
committed resolution nobody has published, or a conflict you or an agent are part-way
through. Provisioning a sync worktree force-removes any old one, and nothing but the
files themselves records that six of eight conflicts were cleared. Finish it, or abort
it — throwing that work away stays available, it just has to be meant.

## After a resolution

Depending on **Review before push** (Workspace Settings), a resolution either goes
straight to origin or waits on the branch for you to read. A held one offers **Review
diff**, **Publish** and **Discard merge**. A resolution whose run is still going never
waits, because nobody is there to look at it.

## Preventing conflicts

- Use well-defined module boundaries
- Configure branch prefix conventions to avoid overlapping changes
- Run sub-tasks that target different files or concerns
