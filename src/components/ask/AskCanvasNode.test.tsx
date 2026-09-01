/**
 * The Ask Canvas node card — `data-state` precedence (unresolved always wins,
 * then selected over cited) and the click contract: a resolved node fires
 * `onActivate`, an unresolved one carries no click handler at all.
 */
import { render, screen, cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { AskCanvasNode } from './AskCanvasNode';
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

const UNRESOLVED_NODE: CanvasNode = { ...NODE, id: 'n2', path: null };

// Non-null path whose stored verdict failed verification — the AC-4 case:
// a path can survive on the node while `CanvasPathVerdict.resolved` is false.
const VERIFIED_FALSE_NODE: CanvasNode = { ...NODE, id: 'n3', path: 'moved/away.rs' };

describe('AskCanvasNode', () => {
  it('renders a distinct data-state for resting, selected, cited, and unresolved', () => {
    const onActivate = vi.fn();
    const { rerender } = render(
      <AskCanvasNode node={NODE} resolved={true} selected={false} cited={false} onActivate={onActivate} />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'resting',
    );

    rerender(
      <AskCanvasNode node={NODE} resolved={true} selected={true} cited={false} onActivate={onActivate} />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'selected',
    );

    rerender(
      <AskCanvasNode node={NODE} resolved={true} selected={false} cited={true} onActivate={onActivate} />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'cited',
    );

    rerender(
      <AskCanvasNode
        node={UNRESOLVED_NODE}
        resolved={false}
        selected={false}
        cited={false}
        onActivate={onActivate}
      />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'unresolved',
    );
  });

  it('selection wins over citation when both are true', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode node={NODE} resolved={true} selected={true} cited={true} onActivate={onActivate} />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'selected',
    );
  });

  it('unresolved wins over selected and cited', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={UNRESOLVED_NODE}
        resolved={false}
        selected={true}
        cited={true}
        onActivate={onActivate}
      />,
    );
    expect(screen.getByText('ExecutionDriver').closest('[data-state]')).toHaveAttribute(
      'data-state',
      'unresolved',
    );
  });

  it('calls onActivate with the node id when a resolved node is clicked', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode node={NODE} resolved={true} selected={false} cited={false} onActivate={onActivate} />,
    );
    fireEvent.click(screen.getByText('ExecutionDriver').closest('[data-state]')!);
    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onActivate).toHaveBeenCalledWith('n1');
  });

  it('does not call onActivate when an unresolved node is clicked', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={UNRESOLVED_NODE}
        resolved={false}
        selected={false}
        cited={false}
        onActivate={onActivate}
      />,
    );
    fireEvent.click(screen.getByText('ExecutionDriver').closest('[data-state]')!);
    expect(onActivate).not.toHaveBeenCalled();
  });

  it('is unresolved and not clickable when the node has a path but its verdict is resolved: false', () => {
    const onActivate = vi.fn();
    render(
      <AskCanvasNode
        node={VERIFIED_FALSE_NODE}
        resolved={false}
        selected={false}
        cited={false}
        onActivate={onActivate}
      />,
    );
    const card = screen.getByText('ExecutionDriver').closest('[data-state]')!;
    expect(card).toHaveAttribute('data-state', 'unresolved');
    fireEvent.click(card);
    expect(onActivate).not.toHaveBeenCalled();
  });
});
