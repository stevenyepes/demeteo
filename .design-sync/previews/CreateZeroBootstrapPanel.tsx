import { CreateZeroBootstrapPanel } from 'demeteo';

const noop = () => {};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-xl">{children}</div>
);

/** Mid-bootstrap — two phases done, one running, the rest pending. */
export const Running = () => (
  <Frame>
    <CreateZeroBootstrapPanel
      phases={[
        { id: 'create_repo', label: 'Create repository', status: 'done' },
        { id: 'create_project', label: 'Register project', status: 'done' },
        { id: 'bootstrap', label: 'Clone and detect strategy', status: 'running' },
        { id: 'save_settings', label: 'Persist project settings', status: 'pending' },
        { id: 'done', label: 'Ready', status: 'pending' },
      ]}
      logs={[
        'created github.com/acme/billing-service-rust (private)',
        'cloning into ~/.demeteo/projects/billing-service-rust',
        'remote: Enumerating objects: 1, done.',
        'detecting default branch… master',
        'detecting test command…',
      ]}
    />
  </Frame>
);

/** Failed — the header flips, the failing phase is marked, and the
 *  error plus a retry CTA appear below the log strip. */
export const Failed = () => (
  <Frame>
    <CreateZeroBootstrapPanel
      canRetry
      onRetry={noop}
      phases={[
        { id: 'create_repo', label: 'Create repository', status: 'done' },
        { id: 'create_project', label: 'Register project', status: 'done' },
        { id: 'bootstrap', label: 'Clone and detect strategy', status: 'error' },
        { id: 'save_settings', label: 'Persist project settings', status: 'pending' },
      ]}
      logs={[
        'created github.com/acme/billing-service-rust (private)',
        'cloning into ~/.demeteo/projects/billing-service-rust',
        'fatal: could not read Username for https://github.com: terminal prompts disabled',
      ]}
      errorMessage="git clone exited 128 — the provider token is missing the `repo` scope"
    />
  </Frame>
);

/** The opening frame, before the first backend event lands. */
export const AwaitingFirstEvent = () => (
  <Frame>
    <CreateZeroBootstrapPanel
      phases={[
        { id: 'create_repo', label: 'Create repository', status: 'running' },
        { id: 'create_project', label: 'Register project', status: 'pending' },
        { id: 'bootstrap', label: 'Clone and detect strategy', status: 'pending' },
        { id: 'save_settings', label: 'Persist project settings', status: 'pending' },
      ]}
      logs={[]}
    />
  </Frame>
);
