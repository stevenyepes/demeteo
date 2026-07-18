// Unit tests for the reusable inline-rename hook
// (`src/hooks/useInlineRename.ts`), extracted from `TerminalTab` so tab/row
// titles can share the renaming behaviour (TERMINALS_VIEW_SPEC §5).
//
// Covers: initial state, entering rename mode, commit-if-changed (and the
// "don't commit if unchanged" rule), cancel, Enter/Escape key handling, and
// upstream `value` sync while not renaming.

import { act, render, renderHook } from '@testing-library/react';
import { useRef, type ReactElement } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { useInlineRename } from './useInlineRename';

// Builds a minimal React.KeyboardEvent stub carrying a spied preventDefault.
function keyEvent(
  key: string,
): { event: React.KeyboardEvent<HTMLInputElement>; preventDefault: ReturnType<typeof vi.fn> } {
  const preventDefault = vi.fn();
  const event = { key, preventDefault } as unknown as React.KeyboardEvent<HTMLInputElement>;
  return { event, preventDefault };
}

describe('useInlineRename — initial state', () => {
  it('starts not renaming with the draft seeded from value', () => {
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit: vi.fn() }),
    );

    expect(result.current.renaming).toBe(false);
    expect(result.current.draft).toBe('alpha');
    expect(result.current.maxLength).toBe(64);
  });

  it('honours a custom maxLength', () => {
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit: vi.fn(), maxLength: 12 }),
    );
    expect(result.current.maxLength).toBe(12);
  });
});

describe('useInlineRename — startRename', () => {
  it('enters rename mode and seeds the draft from value', () => {
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit: vi.fn() }),
    );

    act(() => result.current.startRename());

    expect(result.current.renaming).toBe(true);
    expect(result.current.draft).toBe('alpha');
  });
});

describe('useInlineRename — commitRename', () => {
  it('commits the trimmed new value and exits rename mode', () => {
    const onCommit = vi.fn();
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit }),
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('  beta  '));
    act(() => result.current.commitRename());

    expect(onCommit).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith('beta');
    expect(result.current.renaming).toBe(false);
  });

  it('does NOT call onCommit when the trimmed value is unchanged', () => {
    const onCommit = vi.fn();
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit }),
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('  alpha  '));
    act(() => result.current.commitRename());

    expect(onCommit).not.toHaveBeenCalled();
    expect(result.current.renaming).toBe(false);
    // Draft is reset back to the canonical (untrimmed) committed value.
    expect(result.current.draft).toBe('alpha');
  });
});

describe('useInlineRename — cancelRename', () => {
  it('resets the draft to value and exits rename mode without committing', () => {
    const onCommit = vi.fn();
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit }),
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('beta'));
    act(() => result.current.cancelRename());

    expect(onCommit).not.toHaveBeenCalled();
    expect(result.current.renaming).toBe(false);
    expect(result.current.draft).toBe('alpha');
  });
});

describe('useInlineRename — handleKeyDown', () => {
  it('Enter commits and calls preventDefault', () => {
    const onCommit = vi.fn();
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit }),
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('beta'));

    const { event, preventDefault } = keyEvent('Enter');
    act(() => result.current.handleKeyDown(event));

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(onCommit).toHaveBeenCalledWith('beta');
    expect(result.current.renaming).toBe(false);
  });

  it('Escape cancels and calls preventDefault', () => {
    const onCommit = vi.fn();
    const { result } = renderHook(() =>
      useInlineRename({ value: 'alpha', onCommit }),
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('beta'));

    const { event, preventDefault } = keyEvent('Escape');
    act(() => result.current.handleKeyDown(event));

    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(onCommit).not.toHaveBeenCalled();
    expect(result.current.renaming).toBe(false);
    expect(result.current.draft).toBe('alpha');
  });
});

describe('useInlineRename — upstream value sync', () => {
  it('updates the draft when value changes while not renaming', () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useInlineRename({ value, onCommit: vi.fn() }),
      { initialProps: { value: 'alpha' } },
    );

    expect(result.current.draft).toBe('alpha');

    rerender({ value: 'gamma' });

    expect(result.current.draft).toBe('gamma');
  });

  it('does NOT clobber the draft with an upstream change while renaming', () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useInlineRename({ value, onCommit: vi.fn() }),
      { initialProps: { value: 'alpha' } },
    );

    act(() => result.current.startRename());
    act(() => result.current.setDraft('editing'));

    rerender({ value: 'gamma' });

    expect(result.current.draft).toBe('editing');
  });
});

describe('useInlineRename — focus + select on entering rename', () => {
  it('focuses and selects the bound input when rename mode is entered', () => {
    const selectSpy = vi.fn();
    const controls: { start: () => void } = { start: () => {} };

    function Harness(): ReactElement {
      const rename = useInlineRename({ value: 'alpha', onCommit: vi.fn() });
      controls.start = rename.startRename;
      const ref = useRef<HTMLInputElement | null>(null);
      // Point the hook's inputRef at the real DOM node and spy on select().
      rename.inputRef.current = ref.current;
      return (
        <input
          ref={(node) => {
            ref.current = node;
            rename.inputRef.current = node;
            if (node) node.select = selectSpy;
          }}
          value={rename.renaming ? rename.draft : 'alpha'}
          onChange={(e) => rename.setDraft(e.target.value)}
          readOnly
        />
      );
    }

    render(<Harness />);

    act(() => controls.start());

    expect(selectSpy).toHaveBeenCalled();
  });
});
