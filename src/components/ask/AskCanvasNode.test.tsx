/**
 * The Ask Canvas node card — `data-state` precedence and the click contract.
 *
 * The table under test is the three-way `NodePathState`, not a boolean: a
 * node that never named a file is a normal, clickable card, and only a node
 * whose named file is not there is dimmed. Collapsing those two rendered
 * every `needs_human` node — which by definition names a person — as a ghost.
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskCanvasNode, pathTail } from './AskCanvasNode';
import type { CanvasNode } from '../../types';

afterEach(cleanup);

const NODE: CanvasNode = {
  id: 'n1',
  title: 'ExecutionDriver',
  role: 'orchestration',
  path: 'step_executor/driver.rs',
  stage: 0,
  lane: 0,
};

/** Names a person, not a file — the Gate case. */
const PATHLESS_NODE: CanvasNode = { ...NODE, id: 'n2', path: null };

// Non-null path whose stored verdict failed verification — the AC-4 case:
// a path can survive on the node while `CanvasPathVerdict.resolved` is false.
const VERIFIED_FALSE_NODE: CanvasNode = { ...NODE, id: 'n3', path: 'moved/away.rs' };

function card() {
  return screen.getByText('ExecutionDriver').closest('[data-state]')!;
}

describe('AskCanvasNode', () => {
  it('renders a distinct data-state for resting, selected, cited, and unresolved', () => {
    const onActivate = vi.fn();
    const { rerender } = render(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'resting');

    rerender(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={true}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'selected');

    rerender(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={false}
        cited={true}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'cited');

    rerender(
      <AskCanvasNode
        node={VERIFIED_FALSE_NODE}
        pathState="missing"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'unresolved');
  });

  it('selection wins over citation when both are true', () => {
    render(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={true}
        cited={true}
        x={0}
        y={0}
        onActivate={vi.fn()}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'selected');
  });

  it('a missing path wins over selected and cited', () => {
    render(
      <AskCanvasNode
        node={VERIFIED_FALSE_NODE}
        pathState="missing"
        selected={true}
        cited={true}
        x={0}
        y={0}
        onActivate={vi.fn()}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'unresolved');
  });

  it('calls onActivate with the node id when a resolved node is clicked', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    fireEvent.click(card());
    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onActivate).toHaveBeenCalledWith('n1');
  });

  it('a node that named no file is a normal, clickable card with no path row', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={PATHLESS_NODE}
        pathState="none"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    expect(card()).toHaveAttribute('data-state', 'resting');
    expect(screen.queryByText(/driver\.rs/)).toBeNull();
    fireEvent.click(card());
    expect(onActivate).toHaveBeenCalledWith('n2');
  });

  it('does not call onActivate when a node whose file is missing is clicked', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={VERIFIED_FALSE_NODE}
        pathState="missing"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={onActivate}
      />,
    );
    fireEvent.click(card());
    expect(onActivate).not.toHaveBeenCalled();
  });

  it('shows the tail of a path, keeping the whole of it in the title', () => {
    render(
      <AskCanvasNode
        node={{ ...NODE, path: 'crates/demeteo-core/src/domain/gate/decision.rs' }}
        pathState="resolved"
        selected={false}
        cited={false}
        x={0}
        y={0}
        onActivate={vi.fn()}
      />,
    );
    const row = screen.getByText('gate/decision.rs');
    expect(row).toHaveAttribute('title', 'crates/demeteo-core/src/domain/gate/decision.rs');
  });

  it('positions itself where the layout put it', () => {
    render(
      <AskCanvasNode
        node={NODE}
        pathState="resolved"
        selected={false}
        cited={false}
        x={140}
        y={62}
        onActivate={vi.fn()}
      />,
    );
    expect(card()).toHaveStyle({ left: '140px', top: '62px' });
  });
});

describe('pathTail', () => {
  it('keeps the last two segments, which is the part that names the thing', () => {
    expect(pathTail('crates/demeteo-core/src/adapters/worktree/git_ops')).toBe('worktree/git_ops');
    expect(pathTail('driver.rs')).toBe('driver.rs');
    expect(pathTail('a/b')).toBe('a/b');
  });
});
