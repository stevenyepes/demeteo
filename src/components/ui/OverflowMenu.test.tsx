import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { OverflowMenu } from './OverflowMenu';

describe('OverflowMenu', () => {
  it('runs an item and closes behind it', async () => {
    const onSelect = vi.fn();
    render(<OverflowMenu label="More actions" items={[{ label: 'Cleanup', onSelect }]} />);

    await userEvent.click(screen.getByRole('button', { name: 'More actions' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Cleanup' }));

    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('closes on Escape and hands focus back to its trigger', async () => {
    render(<OverflowMenu label="More actions" items={[{ label: 'Cleanup', onSelect: () => {} }]} />);
    const trigger = screen.getByRole('button', { name: 'More actions' });

    await userEvent.click(trigger);
    // From inside the menu: the trigger already holds focus after the click, so
    // an Escape pressed there could not tell a hand-back from no move at all.
    await userEvent.tab();
    expect(screen.getByRole('menuitem', { name: 'Cleanup' })).toHaveFocus();
    await userEvent.keyboard('{Escape}');

    expect(screen.queryByRole('menu')).toBeNull();
    expect(trigger).toHaveFocus();
  });

  it('closes on a press outside it', async () => {
    render(
      <>
        <button type="button">elsewhere</button>
        <OverflowMenu label="More actions" items={[{ label: 'Cleanup', onSelect: () => {} }]} />
      </>,
    );

    await userEvent.click(screen.getByRole('button', { name: 'More actions' }));
    await userEvent.click(screen.getByRole('button', { name: 'elsewhere' }));

    expect(screen.queryByRole('menu')).toBeNull();
  });
});
