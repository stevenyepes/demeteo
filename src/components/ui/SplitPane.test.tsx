/**
 * The claim under test is a performance one, and it is the reason this
 * component exists rather than the mock's resizer (UI_REDESIGN_PLAN §4.1):
 * dragging the divider must not render React.
 *
 * That matters here specifically because the primary pane holds the
 * ELK-laid-out run graph and `useRunColumnLayout` feeds a `ResizeObserver` on
 * that column into layout planning — so a width that goes through React per
 * pointer move re-plans graph layout per pixel.
 *
 * Two independent tests pin it, because either one alone has a hole. Counting
 * renders of a child catches a *commit* per move, but children arrive as props
 * and React bails out on an unchanged element, so a `useState` inside SplitPane
 * would leave the count flat. The act-warning test closes that hole: a state
 * update from an event dispatched outside `act` is reported by React itself.
 */
import { act, render, screen } from '@testing-library/react';
import { useCallback, useState } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { resizeObserverStubs } from '../../test/setup';
import { SPLIT_SECONDARY_VAR, SplitPane } from './SplitPane';

const MIN_PRIMARY = 480;
const MIN_SECONDARY = 320;
/** Container box every test drags inside: 0..1200, so 320..720 is open. */
const CONTAINER = { left: 0, width: 1200 };

let primaryRenders = 0;
let commits: number[] = [];

function CountingPrimary() {
  primaryRenders += 1;
  return <div data-testid="primary-body">graph</div>;
}

function Harness({ initial }: { initial: number }) {
  const [width, setWidth] = useState(initial);
  const commit = useCallback((next: number) => {
    commits.push(next);
    setWidth(next);
  }, []);

  return (
    <SplitPane
      label="Resize inspector"
      primary={<CountingPrimary />}
      secondary={<div>inspector</div>}
      secondaryWidth={width}
      onSecondaryWidthCommit={commit}
      minPrimary={MIN_PRIMARY}
      minSecondary={MIN_SECONDARY}
    />
  );
}

/** jsdom lays nothing out, so the container's box has to be supplied. */
function stubContainerBox(el: HTMLElement, width: number): void {
  el.getBoundingClientRect = () =>
    ({
      x: CONTAINER.left,
      y: 0,
      left: CONTAINER.left,
      right: CONTAINER.left + width,
      top: 0,
      bottom: 800,
      width,
      height: 800,
      toJSON: () => ({}),
    }) as DOMRect;
}

function mount(initial: number, containerWidth = CONTAINER.width) {
  render(<Harness initial={initial} />);
  const container = screen.getByTestId('split-pane');
  stubContainerBox(container, containerWidth);
  return { container, handle: screen.getByTestId('split-pane-handle') };
}

function secondaryPx(container: HTMLElement): string {
  return container.style.getPropertyValue(SPLIT_SECONDARY_VAR);
}

/** Raw dispatch, not `fireEvent`: outside `act`, so a state update React did
 *  not expect is reported instead of quietly absorbed. */
function pointer(el: HTMLElement, type: string, clientX: number): void {
  el.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 1, button: 0, clientX }));
}

beforeEach(() => {
  primaryRenders = 0;
  commits = [];
});

describe('SplitPane drag', () => {
  it('renders nothing and commits nothing until the pointer is released', () => {
    const { container, handle } = mount(400);
    const rendersBeforeDrag = primaryRenders;

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 760);
      pointer(handle, 'pointermove', 700);
      pointer(handle, 'pointermove', 650);
      pointer(handle, 'pointermove', 600);
    });

    expect(primaryRenders).toBe(rendersBeforeDrag);
    expect(commits).toEqual([]);
    expect(secondaryPx(container)).toBe('600px');

    act(() => {
      pointer(handle, 'pointerup', 600);
    });

    expect(commits).toEqual([600]);
  });

  it('reports no React update for a pointer move', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const { handle } = mount(400);

    pointer(handle, 'pointerdown', 800);
    pointer(handle, 'pointermove', 700);
    pointer(handle, 'pointermove', 640);

    expect(consoleError).not.toHaveBeenCalled();
    consoleError.mockRestore();
  });

  it('tracks the divider position in aria-valuenow while dragging', () => {
    const { handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 700);
    });

    expect(handle).toHaveAttribute('aria-valuenow', '500');
  });

  it('clamps to the width the primary pane minimum leaves', () => {
    const { container, handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 100);
      pointer(handle, 'pointerup', 100);
    });

    expect(secondaryPx(container)).toBe('720px');
    expect(commits).toEqual([720]);
  });

  it('collapses instead of leaving a sliver', () => {
    const { container, handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 1150);
      pointer(handle, 'pointerup', 1150);
    });

    expect(secondaryPx(container)).toBe('0px');
    expect(commits).toEqual([0]);
  });

  it('commits a drag that ends by losing the pointer instead of releasing it', () => {
    const { handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 700);
      handle.dispatchEvent(new PointerEvent('lostpointercapture', { bubbles: true, pointerId: 1 }));
    });

    expect(commits).toEqual([500]);
  });

  it('commits once, not again on the lost-capture that follows a release', () => {
    const { handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 700);
      pointer(handle, 'pointerup', 700);
      handle.dispatchEvent(new PointerEvent('lostpointercapture', { bubbles: true, pointerId: 1 }));
    });

    expect(commits).toEqual([500]);
  });

  it('leaves the committed width in place when the pointer is cancelled', () => {
    const { container, handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointermove', 700);
      handle.dispatchEvent(new PointerEvent('pointercancel', { bubbles: true, pointerId: 1 }));
    });

    expect(commits).toEqual([]);
    expect(secondaryPx(container)).toBe('400px');
  });

  it('focuses the divider it just started dragging, so the arrow keys follow', () => {
    const { handle } = mount(400);

    act(() => {
      pointer(handle, 'pointerdown', 800);
      pointer(handle, 'pointerup', 800);
    });

    expect(handle).toHaveFocus();
  });

  it('ignores moves that arrive without a drag', () => {
    const { container, handle } = mount(400);

    act(() => {
      pointer(handle, 'pointermove', 600);
    });

    expect(secondaryPx(container)).toBe('400px');
    expect(commits).toEqual([]);
  });
});

describe('SplitPane keyboard', () => {
  it('exposes the divider as a separator with its value range', () => {
    const { handle } = mount(400);

    expect(handle).toHaveAttribute('role', 'separator');
    expect(handle).toHaveAttribute('aria-orientation', 'vertical');
    expect(handle).toHaveAttribute('aria-label', 'Resize inspector');
    expect(handle).toHaveAttribute('aria-valuenow', '400');
    expect(handle).toHaveAttribute('aria-valuemin', '0');
    expect(handle).toHaveAttribute('tabindex', '0');
  });

  it('reports the widest secondary pane once the container has been measured', () => {
    const { handle } = mount(400);

    expect(handle).not.toHaveAttribute('aria-valuemax');

    act(() => {
      resizeObserverStubs[0]?.trigger();
    });

    expect(handle).toHaveAttribute('aria-valuemax', '720');
  });

  function press(handle: HTMLElement, key: string): void {
    act(() => {
      handle.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key, cancelable: true }));
    });
  }

  it('nudges the divider with the arrow keys', () => {
    const { container, handle } = mount(400);

    press(handle, 'ArrowLeft');
    expect(commits).toEqual([424]);
    expect(secondaryPx(container)).toBe('424px');

    press(handle, 'ArrowRight');
    expect(commits).toEqual([424, 400]);
  });

  it('jumps to collapsed with Home and to the widest secondary with End', () => {
    const { handle } = mount(400);

    press(handle, 'Home');
    expect(commits).toEqual([0]);

    press(handle, 'End');
    expect(commits).toEqual([0, 720]);
  });

  it('restores the last open width when reopened', () => {
    const { container, handle } = mount(560);

    press(handle, 'Home');
    expect(secondaryPx(container)).toBe('0px');

    press(handle, 'Enter');
    expect(commits).toEqual([0, 560]);
    expect(secondaryPx(container)).toBe('560px');
  });

  it('leaves keys it does not own to the rest of the app', () => {
    const { handle } = mount(400);

    press(handle, 'Tab');

    expect(commits).toEqual([]);
  });
});

describe('SplitPane against a container that shrank', () => {
  it('holds the primary pane minimum against a width committed for a wider window', () => {
    const { container, handle } = mount(400, 700);

    act(() => {
      resizeObserverStubs[0]?.trigger();
    });

    expect(secondaryPx(container)).toBe('220px');
    expect(handle).toHaveAttribute('aria-valuenow', '220');
    expect(commits).toEqual([]);
  });
});

describe('SplitPane collapsed pane', () => {
  it('takes the collapsed secondary pane out of reach', () => {
    mount(0);

    expect(screen.getByTestId('split-pane-secondary')).toHaveAttribute('inert');
  });

  it('leaves an open secondary pane reachable', () => {
    mount(400);

    expect(screen.getByTestId('split-pane-secondary')).not.toHaveAttribute('inert');
  });
});
