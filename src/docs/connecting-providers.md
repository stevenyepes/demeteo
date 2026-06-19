# Connecting Providers

Providers are Git hosting services that Demeteo integrates with for cloning repositories, creating branches, and publishing merge requests.

## Supported Providers

### GitHub (github.com or GitHub Enterprise)

1. Generate a PAT at **Settings → Developer settings → Personal access tokens → Fine-grained tokens**
2. Required scopes: `contents:write`, `pull_requests:write`, `metadata:read`
3. Enter the token and your username in the Provider form

### GitLab (gitlab.com or self-hosted)

1. Generate a PAT at **Settings → Access Tokens**
2. Required scopes: `api`, `write_repository`, `read_repository`
3. Enter the token and your username in the Provider form

## Managing Providers

The **Providers** page shows all connected providers. From here you can:

- **Edit** — update credentials or host URL
- **Disconnect** — remove the provider and revoke access
- **View** — see which repos are accessible through each provider

## Multiple Providers

You can connect multiple providers simultaneously. Each project maps to repositories from one or more providers. The bootstrap phase fetches repo lists from all connected providers and lets you select which ones to include.
