import { useCallback, useEffect, useRef, useState } from 'react';

const DEFAULT_MAX_LENGTH = 64;

export interface UseInlineRenameOptions {
  /** The current committed title. */
  value: string;
  /**
   * Called with the trimmed new value ONLY when it differs from `value`.
   * Committing an unchanged value is a no-op.
   */
  onCommit: (next: string) => void;
  /** Maximum length for the rename input. Defaults to 64. */
  maxLength?: number;
}

export interface UseInlineRename {
  renaming: boolean;
  draft: string;
  setDraft: (v: string) => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
  /** Enter edit mode, seeding the draft from `value`. */
  startRename: () => void;
  /** Trim the draft; if it changed call `onCommit`; exit edit mode. */
  commitRename: () => void;
  /** Reset the draft back to `value` and exit edit mode. */
  cancelRename: () => void;
  /** Enter commits, Escape cancels (both preventDefault). */
  handleKeyDown: (e: React.KeyboardEvent<HTMLInputElement>) => void;
  maxLength: number;
}

/**
 * Reusable inline-rename behaviour shared by tab/row titles (TERMINALS_VIEW_SPEC
 * §5). Extracted from `TerminalTab` so `TerminalTab`, `SessionRow`, and other
 * rows can share the same renaming state, draft handling, auto focus+select on
 * entering rename mode, commit-on-Enter/blur, cancel-on-Escape, and the
 * "don't commit if unchanged" rule.
 */
export function useInlineRename(opts: UseInlineRenameOptions): UseInlineRename {
  const { value, onCommit, maxLength = DEFAULT_MAX_LENGTH } = opts;

  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(value);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Sync the draft when the upstream value changes (e.g. an external rename
  // round-tripped) while we are not actively editing.
  useEffect(() => {
    if (!renaming) setDraft(value);
  }, [value, renaming]);

  // Auto-focus + select the rename input the moment we enter rename mode.
  useEffect(() => {
    if (renaming && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [renaming]);

  const startRename = useCallback(() => {
    setDraft(value);
    setRenaming(true);
  }, [value]);

  const commitRename = useCallback(() => {
    const trimmed = draft.trim();
    setRenaming(false);
    if (trimmed === value) {
      setDraft(value);
      return;
    }
    onCommit(trimmed);
  }, [draft, value, onCommit]);

  const cancelRename = useCallback(() => {
    setDraft(value);
    setRenaming(false);
  }, [value]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        commitRename();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        cancelRename();
      }
    },
    [commitRename, cancelRename],
  );

  return {
    renaming,
    draft,
    setDraft,
    inputRef,
    startRename,
    commitRename,
    cancelRename,
    handleKeyDown,
    maxLength,
  };
}
