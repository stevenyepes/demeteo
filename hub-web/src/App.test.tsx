import { render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import App from './App';

afterEach(() => {
  window.location.hash = '';
});

const NAV_HREFS = ['#/fleet', '#/runs', '#/dispatch', '#/add-instance'];

const ROUTES: { hash: string; heading: string; activeHref: string }[] = [
  { hash: '#/fleet', heading: 'Fleet', activeHref: '#/fleet' },
  { hash: '#/instances/inst-7', heading: 'Instance', activeHref: '#/fleet' },
  { hash: '#/runs', heading: 'Runs', activeHref: '#/runs' },
  { hash: '#/instances/inst-7/runs/feat-42', heading: 'Run', activeHref: '#/runs' },
  { hash: '#/dispatch', heading: 'Dispatch', activeHref: '#/dispatch' },
  { hash: '#/add-instance', heading: 'Add instance', activeHref: '#/add-instance' },
];

function renderAt(hash: string) {
  window.location.hash = hash;
  return render(<App />);
}

function navLinks(): HTMLElement[] {
  return within(screen.getByRole('navigation')).getAllByRole('link');
}

function currentNavHrefs(): (string | null)[] {
  return navLinks()
    .filter((link) => link.hasAttribute('aria-current'))
    .map((link) => link.getAttribute('href'));
}

function monoTexts(container: HTMLElement): (string | null)[] {
  return Array.from(container.querySelectorAll('.font-mono'), (element) => element.textContent);
}

describe('App shell', () => {
  it.each(ROUTES)('$hash renders the $heading view in a glass card', ({ hash, heading }) => {
    renderAt(hash);

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveAccessibleName(heading);
    expect(headings[0]).toHaveClass('font-heading');
    expect(headings[0].closest('.glass-panel')).not.toBeNull();
  });

  it.each(ROUTES)('$hash marks $activeHref as the current nav link', ({ hash, activeHref }) => {
    renderAt(hash);

    expect(currentNavHrefs()).toEqual([activeHref]);
    expect(navLinks().find((link) => link.getAttribute('href') === activeHref)).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  it('links to the four top-level views from a named navigation landmark', () => {
    renderAt('#/fleet');

    expect(screen.getByRole('navigation')).toHaveAccessibleName();
    expect(navLinks().map((link) => link.getAttribute('href'))).toEqual(NAV_HREFS);
  });

  it('swaps the view when the hash changes, keeping the shell mounted', async () => {
    renderAt('#/fleet');
    const nav = screen.getByRole('navigation');
    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Fleet');

    window.location.hash = '#/runs';

    // jsdom queues `hashchange` as a task, so the view is still Fleet on the
    // line after the assignment.
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Runs'),
    );
    expect(currentNavHrefs()).toEqual(['#/runs']);
    expect(screen.getByRole('navigation')).toBe(nav);
  });

  it('replaces the view when only the instance id in the hash changes', async () => {
    const { container } = renderAt('#/instances/inst-7');
    const heading = screen.getByRole('heading', { level: 1 });

    window.location.hash = '#/instances/inst-8';

    await waitFor(() => expect(monoTexts(container)).toEqual(['inst-8']));
    expect(screen.getByRole('heading', { level: 1 })).not.toBe(heading);
  });

  it('keeps the view when the hash moves to another spelling of the same route', async () => {
    const { container } = renderAt('#/instances/inst-7');
    const heading = screen.getByRole('heading', { level: 1 });
    const onHashChange = vi.fn();
    window.addEventListener('hashchange', onHashChange, { once: true });

    window.location.hash = '#/instances/inst-7/';

    // Nothing rendered differs between the two spellings, so the queued
    // `hashchange` itself is the only thing there is to wait for.
    await waitFor(() => expect(onHashChange).toHaveBeenCalledOnce());
    expect(monoTexts(container)).toEqual(['inst-7']);
    expect(screen.getByRole('heading', { level: 1 })).toBe(heading);
  });

  it('renders the not-found view, with no current nav link, for an unknown hash', () => {
    const { container } = renderAt('#/nowhere');

    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Not found');
    expect(monoTexts(container)).toEqual(['/nowhere']);
    expect(currentNavHrefs()).toEqual([]);
    expect(navLinks().map((link) => link.getAttribute('href'))).toEqual(NAV_HREFS);
    expect(
      within(screen.getByRole('main')).getByRole('link', { name: 'Back to Fleet' }),
    ).toHaveAttribute('href', '#/fleet');
  });

  it('passes the instance id from the hash to the Instance view', () => {
    const { container } = renderAt('#/instances/inst-7');

    expect(monoTexts(container)).toEqual(['inst-7']);
  });

  it('passes the instance id and the feature id from the hash to the Run view', () => {
    const { container } = renderAt('#/instances/inst-7/runs/feat-42');

    expect(monoTexts(container)).toEqual(['inst-7', 'feat-42']);
  });
});
