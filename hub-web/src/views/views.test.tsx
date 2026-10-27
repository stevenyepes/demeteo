import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactElement } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HubApiError } from '../lib/api';
import type { Session } from '../lib/auth';
import { WebAuthnCancelledError, WebAuthnUnavailableError } from '../lib/webauthn';
import { AddInstance } from './AddInstance';
import { ApprovePairing } from './ApprovePairing';
import { Dispatch } from './Dispatch';
import { Fleet } from './Fleet';
import { Instance } from './Instance';
import { NotFound } from './NotFound';
import { Passkeys } from './Passkeys';
import { Run } from './Run';
import { Runs } from './Runs';
import { SignIn } from './SignIn';
import { SignUp } from './SignUp';

const auth = vi.hoisted(() => ({
  signUp: vi.fn<(label: string) => Promise<unknown>>(),
  signIn: vi.fn<() => Promise<unknown>>(),
}));

vi.mock('../lib/auth', () => auth);

/**
 * The list never arrives: this file asserts on a view's first paint, and
 * `Passkeys.test.tsx` is where the view meets the Hub's answers.
 */
vi.mock('../lib/passkeys', () => ({ listPasskeys: () => new Promise<never>(() => {}) }));

/** Held in flight for the same reason; `ApprovePairing.test.tsx` is where it meets the prompt. */
vi.mock('../lib/pairing', () => ({ startPairing: () => new Promise<never>(() => {}) }));

/** Held in flight too; `Fleet.test.tsx` is where the views meet the fleet and its instances. */
vi.mock('../lib/fleet', () => ({
  getFleet: () => new Promise<never>(() => {}),
  getInstance: () => new Promise<never>(() => {}),
}));

/** Held in flight too; `Runs.test.tsx` is where the runs list meets the Hub's answers. */
vi.mock('../lib/runs', () => ({
  listRuns: () => new Promise<never>(() => {}),
  getRunDetail: () => new Promise<never>(() => {}),
}));

/** Held in flight as well; `Dispatch.test.tsx` is where the Dispatch view meets them. */
vi.mock('../lib/dispatch', () => ({
  dispatchFeature: () => new Promise<never>(() => {}),
  getRequestStatus: () => new Promise<never>(() => {}),
  dispatchErrorSentence: () => null,
  refusalSentence: () => null,
}));

vi.mock('../lib/attachments', () => ({
  MAX_ATTACHMENTS_PER_DISPATCH: 10,
  uploadAttachment: () => new Promise<never>(() => {}),
}));

const SESSION: Session = { csrfToken: 'csrf-1', credentialId: 'cred-1' };

const VIEWS: { heading: string; element: ReactElement }[] = [
  { heading: 'Fleet', element: <Fleet /> },
  { heading: 'Runs', element: <Runs /> },
  { heading: 'Dispatch', element: <Dispatch /> },
  { heading: 'Add instance', element: <AddInstance /> },
  { heading: 'Instance', element: <Instance instanceId="inst-7" /> },
  { heading: 'Run', element: <Run instanceId="inst-7" featureId="feat-42" /> },
  { heading: 'Not found', element: <NotFound path="/nowhere" /> },
  { heading: 'Sign up', element: <SignUp onSession={() => {}} /> },
  { heading: 'Sign in', element: <SignIn onSession={() => {}} /> },
  { heading: 'Passkeys', element: <Passkeys /> },
  { heading: 'Pair instance', element: <ApprovePairing request="eyJyZXF1ZXN0IjoiMSJ9" /> },
];

function monoTexts(container: HTMLElement): (string | null)[] {
  return Array.from(container.querySelectorAll('.font-mono'), (element) => element.textContent);
}

describe('placeholder views', () => {
  it.each(VIEWS)('$heading renders its heading in the glass card', ({ heading, element }) => {
    render(element);

    const headings = screen.getAllByRole('heading', { level: 1 });
    expect(headings).toHaveLength(1);
    expect(headings[0]).toHaveTextContent(new RegExp(`^${heading}$`));
    expect(headings[0]).toHaveClass('font-heading');
    expect(headings[0].closest('.glass-panel')).not.toBeNull();
  });

  it('Instance shows its instance id in monospace', () => {
    const { container } = render(<Instance instanceId="inst-7" />);

    expect(monoTexts(container)).toEqual(['inst-7']);
  });

  it('NotFound shows the unmatched path and links back to Fleet', () => {
    const { container } = render(<NotFound path="/nowhere" />);

    expect(monoTexts(container)).toEqual(['/nowhere']);
    expect(screen.getByRole('link')).toHaveAttribute('href', '#/fleet');
  });

  it('renders an id that looks like markup as text', () => {
    const { container } = render(<Instance instanceId="<b>x</b>" />);

    expect(monoTexts(container)).toEqual(['<b>x</b>']);
    expect(container.querySelector('b')).toBeNull();
  });
});

type Ceremony = {
  view: string;
  call: typeof auth.signUp | typeof auth.signIn;
  /** Renders the view and brings it to the point where its ceremony has been asked for. */
  begin: (onSession: (session: Session) => void) => Promise<void>;
};

const CEREMONIES: Ceremony[] = [
  {
    view: 'SignUp',
    call: auth.signUp,
    begin: async (onSession) => {
      render(<SignUp onSession={onSession} />);
      await userEvent.type(screen.getByLabelText('Passkey name'), 'laptop');
      await userEvent.click(screen.getByRole('button'));
    },
  },
  {
    view: 'SignIn',
    call: auth.signIn,
    begin: async (onSession) => {
      render(<SignIn onSession={onSession} />);
      await userEvent.click(screen.getByRole('button'));
    },
  },
];

describe('sign-up and sign-in views', () => {
  /** Unscripted, either call rejects: a view that asked for the wrong ceremony fails on that. */
  beforeEach(() => {
    auth.signUp.mockRejectedValue(new Error('signUp was not scripted'));
    auth.signIn.mockRejectedValue(new Error('signIn was not scripted'));
  });

  it('SignUp calls signUp with the trimmed label, then hands the session to onSession', async () => {
    auth.signUp.mockResolvedValueOnce(SESSION);
    const onSession = vi.fn();
    render(<SignUp onSession={onSession} />);

    await userEvent.type(screen.getByLabelText('Passkey name'), '  laptop  ');
    await userEvent.click(screen.getByRole('button'));

    expect(auth.signUp.mock.calls).toEqual([['laptop']]);
    expect(onSession.mock.calls).toEqual([[SESSION]]);
    expect(auth.signIn).not.toHaveBeenCalled();
  });

  it.each([
    { typed: '', why: 'an empty label' },
    { typed: '   ', why: 'a label of spaces' },
    { typed: 'x'.repeat(65), why: 'a label of 65 characters' },
  ])('SignUp cannot be submitted with $why', ({ typed }) => {
    const { container } = render(<SignUp onSession={() => {}} />);
    const form = container.querySelector('form');
    if (form === null) throw new Error('SignUp rendered no form');

    fireEvent.change(screen.getByLabelText('Passkey name'), { target: { value: typed } });

    expect(screen.getByRole('button')).toBeDisabled();
    fireEvent.submit(form);
    expect(auth.signUp).not.toHaveBeenCalled();
  });

  it('SignUp takes a label of 64 characters, counting an emoji as one', () => {
    render(<SignUp onSession={() => {}} />);

    fireEvent.change(screen.getByLabelText('Passkey name'), {
      target: { value: `${'x'.repeat(63)}🔑` },
    });

    expect(screen.getByRole('button')).toBeEnabled();
  });

  it('SignIn calls signIn, then hands the session to onSession', async () => {
    auth.signIn.mockResolvedValueOnce(SESSION);
    const onSession = vi.fn();
    render(<SignIn onSession={onSession} />);

    await userEvent.click(screen.getByRole('button'));

    expect(auth.signIn.mock.calls).toEqual([[]]);
    expect(onSession.mock.calls).toEqual([[SESSION]]);
    expect(auth.signUp).not.toHaveBeenCalled();
  });

  it.each(CEREMONIES)('$view disables its action while the ceremony runs', async ({ call, begin }) => {
    let finish: (session: Session) => void = () => {};
    call.mockReturnValueOnce(
      new Promise<Session>((resolve) => {
        finish = resolve;
      }),
    );
    const onSession = vi.fn();

    await begin(onSession);

    expect(screen.getByRole('button')).toBeDisabled();
    expect(onSession).not.toHaveBeenCalled();

    finish(SESSION);
    expect(await screen.findByRole('button', { name: /passkey$/ })).toBeEnabled();
    expect(onSession.mock.calls).toEqual([[SESSION]]);
  });

  describe.each(CEREMONIES)('$view, when the ceremony fails', ({ call, begin }) => {
    it.each([
      {
        case: 'WebAuthn being unavailable',
        failure: new WebAuthnUnavailableError(),
        shown: /^This browser does not offer passkeys here\./,
      },
      {
        case: 'a cancelled ceremony',
        failure: new WebAuthnCancelledError(),
        shown: /^The passkey prompt was cancelled, timed out, or the authenticator refused\./,
      },
      {
        case: 'a Hub refusal as its code',
        failure: new HubApiError('/v1/x answered HTTP 409', '/v1/x', 409, 'ceremony_expired'),
        shown: /^ceremony_expired$/,
      },
      {
        case: 'a Hub failure without a code as its message',
        failure: new HubApiError('Request to /v1/x failed: offline', '/v1/x', null),
        shown: /^Request to \/v1\/x failed: offline$/,
      },
    ])('renders $case as an error and does not call onSession', async ({ failure, shown }) => {
      call.mockRejectedValueOnce(failure);
      const onSession = vi.fn();

      await begin(onSession);

      expect(await screen.findByRole('alert')).toHaveTextContent(shown);
      expect(onSession).not.toHaveBeenCalled();
      expect(screen.getByRole('button')).toBeEnabled();
    });
  });

  it('a second attempt clears the error of the first', async () => {
    auth.signIn.mockRejectedValueOnce(new WebAuthnCancelledError()).mockResolvedValueOnce(SESSION);
    const onSession = vi.fn();
    render(<SignIn onSession={onSession} />);

    await userEvent.click(screen.getByRole('button'));
    await screen.findByRole('alert');
    await userEvent.click(screen.getByRole('button'));

    expect(screen.queryByRole('alert')).toBeNull();
    expect(onSession.mock.calls).toEqual([[SESSION]]);
  });

  it('offers a passkey and no email, password or recovery control', () => {
    const signUp = render(<SignUp onSession={() => {}} />);
    expect(Array.from(signUp.container.querySelectorAll('input'), (input) => input.type)).toEqual(['text']);
    expect(signUp.container.querySelectorAll('button, a, select, textarea')).toHaveLength(1);
    signUp.unmount();

    const signIn = render(<SignIn onSession={() => {}} />);
    expect(signIn.container.querySelectorAll('input, a, select, textarea')).toHaveLength(0);
    expect(signIn.container.querySelectorAll('button')).toHaveLength(1);
  });
});
