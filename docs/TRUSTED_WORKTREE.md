# Trusted worktree contract

> **Contract only.** [`TrustedWorktreePort`](../crates/demeteo-core/src/ports/worktree_ops.rs)
> records the boundary that terminal-worktree creation/removal and
> dependency-cache materialization must cross. It has no caller yet; existing
> `WorktreeOpsPort` behavior is unchanged by this document.

## The decision

Filesystem mutation below a Project's worktree area is authorized by a resolved
Project and Repository, never by a terminal path supplied by a UI, agent, or
transport. Application policy resolves that ownership and host selection into
an opaque `TrustedWorktreeTarget`; only then may an adapter derive a terminal
destination or a feature cache location.

The narrow port deliberately covers only three operations:

- create a terminal worktree;
- remove a terminal worktree; and
- materialize the fixed set of dependency-cache directories into a worktree.

It does not become a general filesystem port, accept arbitrary source or
destination paths, or authorize shared build output. Build/install outputs are
feature-scoped. Content-addressed download caches may be shared only through a
separate, explicit capability, consistent with [decision 18](DECISIONS.md).

## Trusted-root and no-follow invariants

The target host evaluates every path. The desktop must preserve remote path
syntax as data and must not normalize, canonicalize, or otherwise reinterpret
it using the desktop OS.

An implementation must establish an already-existing physical Project root,
then derive all descendants from it component by component. At every component
it enters or creates, it must reject a symlink, junction, mount-like redirect,
or platform reparse point and verify that the physical location remains the
expected child. It must not use a check-then-follow sequence such as `mkdir -p`,
recursive deletion, or a later path resolution that can be redirected after the
check. Terminal removal additionally re-derives its relative destination and
confirms Git currently registers it as a terminal-owned worktree before it can
remove anything.

The same rule applies to cache materialization. Cache roots and worktree roots
come from Demeteo derivation, the directory set is fixed by the contract, and
the adapter must not traverse an existing link at either endpoint. A cache link
that cannot be created safely is an error, not permission to fall back to an
arbitrary caller path.

## Transport parity

`TrustedWorktreePort` is an `ExecutionPort`-observable contract. Local
subprocess, desktop-over-SSH, and `demeteo-runner` must derive the same eligible
paths, reject the same unsafe structures, and return equivalent success and
failure semantics for identical target-host state. Calling code must not branch
on transport: a difference belongs in the adapter or this contract.

Conformance coverage belongs with the existing execution-parity suites when an
implementation lands. It must exercise real filesystem links/reparse-point
substitutions on each supported transport, including the check-to-use window;
unit tests of path strings alone are not evidence that the no-follow guarantee
holds.

### SSH availability

Desktop-over-SSH currently fails these operations closed. The target host needs
a deployed trusted-worktree helper that performs each operation in one remote
transaction; independently issued SFTP calls or commands cannot preserve the
no-follow proof across the check-to-use window. Until that helper ships, the
SSH adapter returns an explicit unsupported error instead of attempting a
best-effort fallback. The local backend and runner are not affected.

## Related

- [Execution parity](EXECUTION_PARITY.md) — cross-transport behavioral rule
- [Architecture](ARCHITECTURE.md) — port boundary and adapter layout
- [Decision 18](DECISIONS.md) — concurrent features and cache isolation
