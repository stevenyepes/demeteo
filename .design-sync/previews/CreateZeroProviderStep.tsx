import { CreateZeroProviderStep } from 'demeteo';

const noop = () => {};

const PROVIDERS = [
  { id: 'p-gh', type: 'github', name: 'GitHub', host: 'github.com', pat: '', username: 'acme-bot', avatarUrl: '' },
  { id: 'p-gl', type: 'gitlab', name: 'GitLab (self-hosted)', host: 'git.acme.internal', pat: '', username: 'acme-bot', avatarUrl: '' },
];

const NAMESPACES = [
  { id: 'ns-acme', name: 'acme', kind: 'org' as const },
  { id: 'ns-platform', name: 'acme/platform', kind: 'group' as const },
  { id: 'ns-personal', name: 'acme-bot', kind: 'personal' as const },
];

// Mirrors the wizard's own slug rule closely enough to exercise the
// inline error path without importing a non-exported helper.
const validateSlug = (v: string) =>
  v && !/^[a-z0-9][a-z0-9._-]*$/.test(v)
    ? 'Use lowercase letters, digits, dots, dashes or underscores.'
    : '';

const base = {
  projectName: 'billing-service-rust',
  providers: PROVIDERS,
  namespaces: NAMESPACES,
  namespacesLoading: false,
  onProviderChange: noop,
  onNamespaceChange: noop,
  onSlugChange: noop,
  onPrivateChange: noop,
  validateSlug,
};

const Frame = ({ children }: { children: React.ReactNode }) => (
  <div className="w-full max-w-2xl">{children}</div>
);

/** Everything chosen — provider, namespace, slug, private visibility. */
export const FullySelected = () => (
  <Frame>
    <CreateZeroProviderStep
      {...base}
      providerId="p-gh"
      namespaceId="ns-platform"
      repoSlug="billing-service-rust"
      repoPrivate
    />
  </Frame>
);

/** Before a provider is picked, the namespace row says so. */
export const NoProviderYet = () => (
  <Frame>
    <CreateZeroProviderStep {...base} providerId="" namespaceId="" repoSlug="" repoPrivate />
  </Frame>
);

/** Namespaces are being fetched from the provider. */
export const FetchingNamespaces = () => (
  <Frame>
    <CreateZeroProviderStep
      {...base}
      namespaces={[]}
      namespacesLoading
      providerId="p-gl"
      namespaceId=""
      repoSlug=""
      repoPrivate={false}
    />
  </Frame>
);

/** An invalid slug surfaces an inline amber message under the input. */
export const InvalidSlug = () => (
  <Frame>
    <CreateZeroProviderStep
      {...base}
      providerId="p-gh"
      namespaceId="ns-acme"
      repoSlug="Billing Service"
      repoPrivate={false}
    />
  </Frame>
);

/** No providers connected at all — the amber pointer to Settings. */
export const NoProviders = () => (
  <Frame>
    <CreateZeroProviderStep
      {...base}
      providers={[]}
      namespaces={[]}
      providerId=""
      namespaceId=""
      repoSlug=""
      repoPrivate
    />
  </Frame>
);
