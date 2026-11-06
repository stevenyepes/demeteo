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

  it('lists the three desktop pairing steps in order', () => {
    render(<AddInstance />);

    const steps = within(screen.getByRole('list', { name: 'A desktop' })).getAllByRole('listitem');
    expect(steps).toHaveLength(3);
    expect(steps[0]).toHaveTextContent(/Settings › Hub/);
    expect(steps[1]).toHaveTextContent(/URL/);
    expect(steps[2]).toHaveTextContent(/fingerprint/);
  });

  it("gives a runner host the pair command with this Hub's URL, and sends its code to the device page", () => {
    render(<AddInstance />);

    const steps = within(screen.getByRole('list', { name: 'A runner host' })).getAllByRole('listitem');
    expect(steps).toHaveLength(3);
    expect(steps[0].querySelector('.font-mono')?.textContent).toBe(
      `demeteo-runner hub pair ${window.location.origin}`,
    );
    expect(within(steps[1]).getByRole('link', { name: 'Pair runner' })).toHaveAttribute('href', '#/device');
    expect(steps[2]).toHaveTextContent(/fingerprint/);
  });

  it("shows this Hub's URL from the page origin, in monospace", () => {
    render(<AddInstance />);

    const desktop = screen.getByRole('list', { name: 'A desktop' });
    expect(desktop.querySelector('li .font-mono')?.textContent).toBe(window.location.origin);
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

    it('copies the pairing command a runner host runs', async () => {
      const writeText = vi.fn(() => Promise.resolve());
      Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
      render(<AddInstance />);

      await userEvent.click(screen.getByRole('button', { name: 'Copy the pairing command' }));

      expect(writeText.mock.calls).toEqual([[`demeteo-runner hub pair ${window.location.origin}`]]);
    });

    it('offers no copy control where the browser has no clipboard', () => {
      Reflect.deleteProperty(navigator, 'clipboard');
      render(<AddInstance />);

      expect(screen.queryByRole('button')).toBeNull();
    });
  });
});
