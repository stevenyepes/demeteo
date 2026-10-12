import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import App from './App';
import { setCsrfToken } from './lib/api';

afterEach(() => {
  vi.unstubAllGlobals();
  setCsrfToken(null);
  window.location.hash = '';
});

type Answers = Record<string, () => Response | Promise<Response>>;

/**
 * Answers the requests it was given, by method and path, and throws on any
 * other: a view that reaches the Hub from behind the session gate fails the
 * test that rendered it instead of being handed an empty success.
 */
function stubHub(answers: Answers): string[] {
  const asked: string[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const request = `${init?.method ?? 'GET'} ${String(input)}`;
      asked.push(request);
      const answer = answers[request];
      if (answer === undefined) throw new Error(`fetch double was not told about ${request}`);
      return answer();
    }),
  );
  return asked;
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

const SIGNED_IN: Answers = {
  'GET /v1/session': () => json({ csrf_token: 'csrf-1', credential_id: 'cred-1' }),
  'GET /v1/passkeys': () => json({ passkeys: [] }),
};

const SIGNED_OUT: Answers = {
  'GET /v1/session': () => json({ error: 'unauthenticated' }, 401),
};

const NAV_HREFS = ['#/fleet', '#/runs', '#/dispatch', '#/add-instance', '#/passkeys'];

const ROUTES: { hash: string; heading: string; activeHref: string }[] = [
  { hash: '#/fleet', heading: 'Fleet', activeHref: '#/fleet' },
  { hash: '#/instances/inst-7', heading: 'Instance', activeHref: '#/fleet' },
  { hash: '#/runs', heading: 'Runs', activeHref: '#/runs' },
  { hash: '#/instances/inst-7/runs/feat-42', heading: 'Run', activeHref: '#/runs' },
  { hash: '#/dispatch', heading: 'Dispatch', activeHref: '#/dispatch' },
  { hash: '#/add-instance', heading: 'Add instance', activeHref: '#/add-instance' },
  { hash: '#/passkeys', heading: 'Passkeys', activeHref: '#/passkeys' },
  { hash: '#/sign-in', heading: 'Fleet', activeHref: '#/fleet' },
  { hash: '#/sign-up', heading: 'Fleet', activeHref: '#/fleet' },
];

async function renderAt(hash: string) {
  stubHub(SIGNED_IN);
  window.location.hash = hash;
  const rendered = render(<App />);
  await screen.findByRole('navigation');
  return rendered;
}

async function renderSignedOutAt(hash: string) {
  const asked = stubHub(SIGNED_OUT);
  window.location.hash = hash;
  const rendered = render(<App />);
  await screen.findByRole('heading', { level: 1 });
  return { ...rendered, asked };
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

describe('App shell, signed in', () => {
  it.each(ROUTES)('$hash renders the $heading view in a glass card', async ({ hash, heading }) => {
    await renderAt(hash);

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveAccessibleName(heading);
    expect(headings[0]).toHaveClass('font-heading');
    expect(headings[0].closest('.glass-panel')).not.toBeNull();
  });

  it.each(ROUTES)('$hash marks $activeHref as the current nav link', async ({ hash, activeHref }) => {
    await renderAt(hash);

    expect(currentNavHrefs()).toEqual([activeHref]);
    expect(navLinks().find((link) => link.getAttribute('href') === activeHref)).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  it('links to the five top-level views from a named navigation landmark', async () => {
    await renderAt('#/fleet');

    expect(screen.getByRole('navigation')).toHaveAccessibleName();
    expect(navLinks().map((link) => link.getAttribute('href'))).toEqual(NAV_HREFS);
  });

  it('offers one sign-out control, in the nav', async () => {
    await renderAt('#/fleet');

    expect(screen.getAllByRole('button', { name: 'Sign out' })).toEqual([
      within(screen.getByRole('navigation')).getByRole('button', { name: 'Sign out' }),
    ]);
  });

  it('swaps the view when the hash changes, keeping the shell mounted', async () => {
    await renderAt('#/fleet');
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
    const { container } = await renderAt('#/instances/inst-7');
    const heading = screen.getByRole('heading', { level: 1 });

    window.location.hash = '#/instances/inst-8';

    await waitFor(() => expect(monoTexts(container)).toEqual(['inst-8']));
    expect(screen.getByRole('heading', { level: 1 })).not.toBe(heading);
  });

  it('keeps the view when the hash moves to another spelling of the same route', async () => {
    const { container } = await renderAt('#/instances/inst-7');
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

  it('renders the not-found view, with no current nav link, for an unknown hash', async () => {
    const { container } = await renderAt('#/nowhere');

    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Not found');
    expect(monoTexts(container)).toEqual(['/nowhere']);
    expect(currentNavHrefs()).toEqual([]);
    expect(navLinks().map((link) => link.getAttribute('href'))).toEqual(NAV_HREFS);
    expect(
      within(screen.getByRole('main')).getByRole('link', { name: 'Back to Fleet' }),
    ).toHaveAttribute('href', '#/fleet');
  });

  it('passes the instance id from the hash to the Instance view', async () => {
    const { container } = await renderAt('#/instances/inst-7');

    expect(monoTexts(container)).toEqual(['inst-7']);
  });

  it('passes the instance id and the feature id from the hash to the Run view', async () => {
    const { container } = await renderAt('#/instances/inst-7/runs/feat-42');

    expect(monoTexts(container)).toEqual(['inst-7', 'feat-42']);
  });
});

const GATED_HASHES = [
  '',
  ...ROUTES.map(({ hash }) => hash).filter((hash) => hash !== '#/sign-up'),
  '#/nowhere',
];

describe('App shell, signed out', () => {
  it.each(GATED_HASHES)('%j renders the sign-in view, no nav, and asks the Hub for nothing else', async (hash) => {
    const { asked } = await renderSignedOutAt(hash);

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveAccessibleName('Sign in');
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Sign out' })).toBeNull();
    expect(asked).toEqual(['GET /v1/session']);
  });

  it('#/sign-up renders the sign-up view and no nav', async () => {
    await renderSignedOutAt('#/sign-up');

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveAccessibleName('Sign up');
    expect(screen.queryByRole('navigation')).toBeNull();
  });

  it('links nowhere but to the other of the two views', async () => {
    await renderSignedOutAt('#/runs');
    expect(screen.getAllByRole('link').map((link) => link.getAttribute('href'))).toEqual(['#/sign-up']);

    await userEvent.click(screen.getByRole('link', { name: 'Sign up' }));

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Sign up'),
    );
    expect(screen.getAllByRole('link').map((link) => link.getAttribute('href'))).toEqual(['#/sign-in']);
    expect(screen.getByRole('link', { name: 'Sign in' })).toBeInTheDocument();
  });

  it('keeps the sign-in view when the hash moves between two routes it stands in for', async () => {
    await renderSignedOutAt('#/fleet');
    const heading = screen.getByRole('heading', { level: 1 });
    const onHashChange = vi.fn();
    window.addEventListener('hashchange', onHashChange, { once: true });

    window.location.hash = '#/runs';

    await waitFor(() => expect(onHashChange).toHaveBeenCalledOnce());
    expect(screen.getByRole('heading', { level: 1 })).toBe(heading);
  });
});

describe('App shell, before the session is known', () => {
  it('renders neither a view nor the nav while the session is being read', () => {
    stubHub({ 'GET /v1/session': () => new Promise<never>(() => {}) });
    window.location.hash = '#/fleet';

    render(<App />);

    expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByRole('link')).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('renders the failure, and not the sign-in view, when the session read is not answered', async () => {
    stubHub({ 'GET /v1/session': () => json({ error: 'internal' }, 500) });
    window.location.hash = '#/fleet';

    render(<App />);

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
  });
});

describe('App shell, signing out', () => {
  it('ends the session, then shows the sign-in view without the nav', async () => {
    const asked = stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => json({}) });
    window.location.hash = '#/runs';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Sign in'),
    );
    expect(screen.queryByRole('navigation')).toBeNull();
    expect(asked).toEqual(['GET /v1/session', 'POST /v1/session/end']);
  });

  it('lands on the sign-in view when the hash was left on #/sign-up', async () => {
    stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => json({}) });
    window.location.hash = '#/sign-up';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Sign in'),
    );
    expect(window.location.hash).toBe('#/sign-in');
  });

  it('disables the control while the Hub has not answered, so one click is one request', async () => {
    const asked = stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => new Promise<never>(() => {}) });
    window.location.hash = '#/runs';
    render(<App />);
    const control = await screen.findByRole('button', { name: 'Sign out' });

    await userEvent.click(control);
    await userEvent.click(control);

    expect(control).toBeDisabled();
    expect(asked).toEqual(['GET /v1/session', 'POST /v1/session/end']);
  });

  it('stays signed in and shows the failure when the Hub does not end the session', async () => {
    stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => json({ error: 'internal' }, 500) });
    window.location.hash = '#/runs';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Runs');
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeEnabled();
    expect(currentNavHrefs()).toEqual(['#/runs']);
  });

  it('takes the failure down on leaving the view, and not on another spelling of the same route', async () => {
    stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => json({ error: 'internal' }, 500) });
    window.location.hash = '#/instances/inst-7';
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);
    const onHashChange = vi.fn();
    window.addEventListener('hashchange', onHashChange, { once: true });

    window.location.hash = '#/instances/inst-7/';

    await waitFor(() => expect(onHashChange).toHaveBeenCalledOnce());
    expect(screen.getByRole('alert')).toHaveTextContent(/^internal$/);

    window.location.hash = '#/runs';

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Runs'),
    );
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: 'Sign out' })).toBeEnabled();
  });

  it('does not bring the failure back on returning to the view it was shown over', async () => {
    stubHub({ ...SIGNED_IN, 'POST /v1/session/end': () => json({ error: 'internal' }, 500) });
    window.location.hash = '#/runs';
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));
    await screen.findByRole('alert');

    window.location.hash = '#/fleet';
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Fleet'),
    );
    window.location.hash = '#/runs';

    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Runs'),
    );
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

const UNAUTHENTICATED = () => json({ error: 'unauthenticated' }, 401);

async function expectSignInView() {
  await waitFor(() =>
    expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Sign in'),
  );
  expect(screen.queryByRole('navigation')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Sign out' })).toBeNull();
  expect(screen.queryByRole('alert')).toBeNull();
}

describe('App shell, when the Hub no longer knows the session', () => {
  it('shows the sign-in view, and no failure, when a read is answered 401', async () => {
    const asked = stubHub({ ...SIGNED_IN, 'GET /v1/passkeys': UNAUTHENTICATED });
    window.location.hash = '#/passkeys';
    render(<App />);

    await expectSignInView();
    expect(asked).toEqual(['GET /v1/session', 'GET /v1/passkeys']);
    expect(window.location.hash).toBe('#/passkeys');
  });

  it('shows the sign-in view, and no failure, when an action is answered 401', async () => {
    const asked = stubHub({ ...SIGNED_IN, 'POST /v1/passkeys/add/start': UNAUTHENTICATED });
    window.location.hash = '#/passkeys';
    render(<App />);

    await userEvent.type(await screen.findByLabelText('Passkey name'), 'phone');
    await userEvent.click(screen.getByRole('button', { name: 'Add passkey' }));

    await expectSignInView();
    expect(asked).toEqual([
      'GET /v1/session',
      'GET /v1/passkeys',
      'POST /v1/passkeys/add/start',
      'GET /v1/passkeys',
    ]);
  });

  it('shows the sign-in view, and no failure, when Sign out is answered 401', async () => {
    const asked = stubHub({ ...SIGNED_IN, 'POST /v1/session/end': UNAUTHENTICATED });
    window.location.hash = '#/runs';
    render(<App />);

    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));

    await expectSignInView();
    expect(asked).toEqual(['GET /v1/session', 'POST /v1/session/end']);
  });

  it('lands on the sign-in view when the 401 arrives with the hash on #/sign-up', async () => {
    let answerList: (response: Response) => void = () => {};
    const asked = stubHub({
      ...SIGNED_IN,
      'GET /v1/passkeys': () =>
        new Promise<Response>((resolve) => {
          answerList = resolve;
        }),
    });
    window.location.hash = '#/passkeys';
    render(<App />);
    await waitFor(() => expect(asked).toContain('GET /v1/passkeys'));
    window.location.hash = '#/sign-up';
    await waitFor(() =>
      expect(screen.getByRole('heading', { level: 1 })).toHaveAccessibleName('Fleet'),
    );

    answerList(UNAUTHENTICATED());

    await expectSignInView();
    expect(window.location.hash).toBe('#/sign-in');
  });

  it('takes a sign-out failure off the page with the session it was about', async () => {
    stubHub({
      ...SIGNED_IN,
      'POST /v1/session/end': () => json({ error: 'internal' }, 500),
      'GET /v1/passkeys': UNAUTHENTICATED,
    });
    window.location.hash = '#/runs';
    render(<App />);
    await userEvent.click(await screen.findByRole('button', { name: 'Sign out' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/^internal$/);

    window.location.hash = '#/passkeys';

    await expectSignInView();
  });
});
