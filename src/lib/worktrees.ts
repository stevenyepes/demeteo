/**
 * The folder a branch gets, so creating a worktree is one decision instead of
 * two.
 *
 * Branch and directory were separate inputs, and nobody typing `feature/x` in
 * one of them wants anything but `feature-x` in the other — the second field
 * was a naming exercise standing between the user and the thing they asked
 * for. It stays overridable, because two branches can legitimately want one
 * name shape (`fix/login` and `fix-login`), and only the person creating them
 * knows which directory they mean.
 *
 * `/` becomes `-` rather than a real subdirectory: the destination is created
 * one component at a time under a fenced parent, and a nested name means more
 * directories to check, to leave behind on removal, and to collide with.
 */
export function deriveWorktreeName(branch: string): string {
  return branch
    .trim()
    .replace(/[^A-Za-z0-9._-]+/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/^[-.]+/, "")
    .replace(/[-.]+$/, "");
}
