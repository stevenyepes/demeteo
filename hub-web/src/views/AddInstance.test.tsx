import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { AddInstance } from './AddInstance';

describe('AddInstance', () => {
  it('offers no text input and no tabs', () => {
    render(<AddInstance />);

    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.queryByRole('tab')).toBeNull();
  });

  it('never mentions a runner or a code', () => {
    const { container } = render(<AddInstance />);

    expect(container.textContent).not.toMatch(/runner/i);
    expect(container.textContent).not.toMatch(/\bcode\b/i);
  });

  it('lists the three desktop pairing steps in order', () => {
    render(<AddInstance />);

    const steps = within(screen.getByRole('list')).getAllByRole('listitem');
    expect(steps).toHaveLength(3);
    expect(steps[0]).toHaveTextContent(/Settings › Hub/);
    expect(steps[1]).toHaveTextContent(/URL/);
    expect(steps[2]).toHaveTextContent(/fingerprint/);
  });

  it("shows this Hub's URL from the page origin, in monospace", () => {
    const { container } = render(<AddInstance />);

    const url = container.querySelector('li .font-mono');
    expect(url?.textContent).toBe(window.location.origin);
  });
});
