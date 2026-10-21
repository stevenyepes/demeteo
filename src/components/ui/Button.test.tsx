import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Button } from './Button';

describe('Button', () => {
  it('is not a submit button unless asked', () => {
    render(
      <form>
        <Button>Cancel</Button>
        <Button type="submit">Save</Button>
      </form>,
    );
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveAttribute('type', 'button');
    expect(screen.getByRole('button', { name: 'Save' })).toHaveAttribute('type', 'submit');
  });

  it('fills the primary with the accent, never a status hue', () => {
    render(<Button variant="primary">Approve</Button>);
    const button = screen.getByRole('button', { name: 'Approve' });
    expect(button).toHaveClass('bg-accent');
    expect(button.className).not.toMatch(/emerald|cyan|violet|shadow/);
  });

  it('does not fire while disabled', () => {
    const onClick = vi.fn();
    render(
      <Button variant="primary" disabled onClick={onClick}>
        Start
      </Button>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(onClick).not.toHaveBeenCalled();
  });
});
