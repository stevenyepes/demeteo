import type { ReactElement } from 'react';

import { HubNav } from './components/HubNav';
import { type Route, routeHref } from './lib/router';
import { useRoute } from './lib/useRoute';
import { AddInstance } from './views/AddInstance';
import { Dispatch } from './views/Dispatch';
import { Fleet } from './views/Fleet';
import { Instance } from './views/Instance';
import { NotFound } from './views/NotFound';
import { Run } from './views/Run';
import { Runs } from './views/Runs';

function viewFor(route: Route): ReactElement {
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
      return <Dispatch />;
    case 'add-instance':
      return <AddInstance />;
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
  return (
    <div className="flex h-screen flex-col">
      <header className="flex shrink-0 flex-wrap items-center gap-x-8 gap-y-2 border-b border-white/5 px-8 py-3">
        <span className="font-heading text-lg font-semibold text-white tracking-tight">Demeteo Hub</span>
        <HubNav route={route} />
      </header>
      {/* `body` is `height: 100vh; overflow: hidden` in `src/App.css`, so a view
          taller than the window is unreachable unless this element scrolls. */}
      <main className="min-h-0 flex-1 overflow-y-auto p-8">
        <div key={viewKey(route)} className="mx-auto w-full max-w-4xl animate-fade-in">
          {viewFor(route)}
        </div>
      </main>
    </div>
  );
}

export default App;
