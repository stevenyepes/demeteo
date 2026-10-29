import { type ReactElement, useCallback, useEffect, useState } from 'react';

import { CeremonyError } from './components/CeremonyError';
import { HubNav } from './components/HubNav';
import { onSessionGone } from './lib/api';
import { fetchSession, signOut } from './lib/auth';
import { parseRoute, type Route, routeHref } from './lib/router';
import { useRoute } from './lib/useRoute';
import { AddInstance } from './views/AddInstance';
import { ApprovePairing } from './views/ApprovePairing';
import { Dispatch } from './views/Dispatch';
import { Fleet } from './views/Fleet';
import { Instance } from './views/Instance';
import { NotFound } from './views/NotFound';
import { Passkeys } from './views/Passkeys';
import { Run } from './views/Run';
import { Runs } from './views/Runs';
import { SignIn } from './views/SignIn';
import { SignUp } from './views/SignUp';

/**
 * `unknown` is its own state, and not signed out: `fetchSession` rejects when
 * the Hub answered anything but a session or a `401`, and the sign-in view put
 * in front of a user who is in fact signed in would be the shell asserting
 * something nobody told it.
 */
type SessionState =
  | { status: 'loading' }
  | { status: 'unknown'; error: unknown }
  | { status: 'signed-out' }
  | { status: 'signed-in' };

/**
 * The session decides the view before the hash does. Signed out, every route
 * but `#/sign-up` is the sign-in view, and the hash is left alone so that a
 * deep link is where signing in lands. Signed in, the two auth routes have
 * nothing left to offer and are Fleet.
 */
function gatedRoute(route: Route, signedIn: boolean): Route {
  if (signedIn) {
    return route.name === 'sign-up' || route.name === 'sign-in' ? { name: 'fleet' } : route;
  }
  return route.name === 'sign-up' ? route : { name: 'sign-in' };
}

const OTHER_AUTH_VIEW = {
  'sign-in': { prompt: 'No passkey on this Hub yet?', route: { name: 'sign-up' }, label: 'Sign up' },
  'sign-up': { prompt: 'Already registered a passkey?', route: { name: 'sign-in' }, label: 'Sign in' },
} as const;

function viewFor(route: Route, onSession: () => void): ReactElement {
  switch (route.name) {
    case 'fleet':
      return <Fleet />;
    case 'instance':
      return <Instance instanceId={route.instanceId} />;
    case 'runs':
      return <Runs />;
    case 'run':
      return <Run instanceId={route.instanceId} featureId={route.featureId} />;
    case 'dispatch':
      return <Dispatch instanceId={route.instanceId} />;
    case 'add-instance':
      return <AddInstance />;
    case 'pair':
      return <ApprovePairing request={route.request} />;
    case 'sign-up':
      return <SignUp onSession={onSession} />;
    case 'sign-in':
      return <SignIn onSession={onSession} />;
    case 'passkeys':
      return <Passkeys />;
    case 'not-found':
      return <NotFound path={route.path} />;
    default: {
      const unhandled: never = route;
      return unhandled;
    }
  }
}

/**
 * Keyed on the parsed route, not on `window.location.hash`: `#/instances/a` and
 * `#/instances/a/` are one place, and a key taken from the hash would discard
 * the view's state on moving between them.
 */
function viewKey(route: Route): string {
  return route.name === 'not-found' ? route.name : routeHref(route);
}

function App() {
  const route = useRoute();
  const [session, setSession] = useState<SessionState>({ status: 'loading' });
  const [signingOut, setSigningOut] = useState(false);
  const [signOutFailure, setSignOutFailure] = useState<{ error: unknown } | null>(null);

  useEffect(() => {
    let unmounted = false;
    fetchSession().then(
      (held) => {
        if (!unmounted) setSession({ status: held === null ? 'signed-out' : 'signed-in' });
      },
      (error: unknown) => {
        if (!unmounted) setSession({ status: 'unknown', error });
      },
    );
    return () => {
      unmounted = true;
    };
  }, []);

  /**
   * Signing up leaves the hash on `#/sign-up`, which signed in is Fleet and
   * signed out is the sign-up view again. The hash is read here, not taken
   * from the render that drew the control, which may be a route behind.
   */
  const showSignedOut = useCallback(() => {
    if (parseRoute(window.location.hash).name === 'sign-up') {
      window.location.hash = routeHref({ name: 'sign-in' });
    }
    setSession({ status: 'signed-out' });
    setSignOutFailure(null);
  }, []);

  useEffect(() => onSessionGone(showSignedOut), [showSignedOut]);

  async function endSession() {
    setSignOutFailure(null);
    setSigningOut(true);
    try {
      await signOut();
    } catch (error) {
      setSignOutFailure({ error });
      return;
    } finally {
      setSigningOut(false);
    }
    showSignedOut();
  }

  const signedIn = session.status === 'signed-in';
  const shown = gatedRoute(route, signedIn);
  const shownKey = viewKey(shown);

  /**
   * A sign-out failure is about the attempt made from the view it appeared
   * over, and is taken down on leaving that view. Compared in render rather
   * than in an effect, so the next view is never drawn under it; and on
   * `viewKey`, so another spelling of the same route is not leaving.
   */
  const [lastShownKey, setLastShownKey] = useState(shownKey);
  if (lastShownKey !== shownKey) {
    setLastShownKey(shownKey);
    setSignOutFailure(null);
  }
  const otherAuthView =
    !signedIn && (shown.name === 'sign-in' || shown.name === 'sign-up') ? OTHER_AUTH_VIEW[shown.name] : null;
  return (
    <div className="flex h-screen flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-8 gap-y-2 border-b border-white/5 px-8 py-3">
        <span className="font-heading text-lg font-semibold text-white tracking-tight">Demeteo Hub</span>
        {signedIn && <HubNav route={shown} signingOut={signingOut} onSignOut={endSession} />}
      </header>
      {/* `body` is `height: 100vh; overflow: hidden` in `src/App.css`, so a view
          taller than the window is unreachable unless this element scrolls. */}
      <main aria-busy={session.status === 'loading'} className="min-h-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-4">
          {session.status === 'unknown' && (
            <>
              <CeremonyError error={session.error} />
              <p className="text-sm text-slate-400">
                The Hub did not say whether you are signed in. Reload the page to ask again.
              </p>
            </>
          )}
          {signOutFailure && <CeremonyError error={signOutFailure.error} />}
          {(signedIn || session.status === 'signed-out') && (
            <div key={shownKey} className="animate-fade-in">
              {viewFor(shown, () => setSession({ status: 'signed-in' }))}
              {otherAuthView && (
                <p className="mt-4 text-sm text-slate-400">
                  {otherAuthView.prompt}{' '}
                  <a
                    href={routeHref(otherAuthView.route)}
                    className="text-accent-ink transition-colors hover:text-cyan-300 focus-visible:text-cyan-300"
                  >
                    {otherAuthView.label}
                  </a>
                </p>
              )}
            </div>
          )}
        </div>
      </main>
    </div>
  );
}

export default App;
