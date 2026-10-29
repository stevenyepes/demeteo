import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

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

  describe('copying the URL', () => {
    afterEach(() => {
      Reflect.deleteProperty(navigator, 'clipboard');
    });

    it("copies this Hub's URL, and says so", async () => {
      const writeText = vi.fn(() => Promise.resolve());
      Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
      render(<AddInstance />);

      await userEvent.click(screen.getByRole('button', { name: "Copy this Hub's URL" }));

      expect(writeText.mock.calls).toEqual([[window.location.origin]]);
      expect(await screen.findByText('Copied')).toBeInTheDocument();
    });

    it('offers no copy control where the browser has no clipboard', () => {
      Reflect.deleteProperty(navigator, 'clipboard');
      render(<AddInstance />);

      expect(screen.queryByRole('button')).toBeNull();
    });
  });
});
