# Changelog

## Unreleased

- Added the Ask Canvas mockup artboards and their documentation index.
- Documented the MCP endpoint in `docs/MCP_INTEGRATION.md`: stateless HTTP transport, the single supported protocol revision, the built-in OAuth 2.1 server, the `read`/`spend`/`configure` scopes, the twelve-tool surface, what is deliberately excluded, and the off-by-default listener.
- Recorded MCP decisions 45–52 in `docs/DECISIONS.md` and the unresolved per-grant project-list question in `docs/OPEN_QUESTIONS.md` §19.
- Fixed stale decision-count anchors and wording in `docs/DECISIONS.md`, `docs/OPEN_QUESTIONS.md` and `AGENTS.md` by renaming "The 44 Decisions" to "The Locked Decisions".
- Documented the Demeteo Hub in the new `docs/HUB.md`: trust model and the list of what never leaves an instance, pairing, the instance-initiated socket, the six request kinds and attachments, scopes and local ceilings, passkey-signed gate decisions, storage and custody, and what is not built or not decided yet.
- Recorded Hub decisions 56–63 in `docs/DECISIONS.md`, with detail blocks for 58, 60 and 61, and pointed the web-companion item in `docs/OPEN_QUESTIONS.md` §17 at them.
- Marked `docs/hub-design/README.md` as decided but not built and listed the mockup parts the decisions supersede; added the Hub's separate gate-decision surface to `docs/MCP_INTEGRATION.md` §8, a Hub section to `docs/ARCHITECTURE.md` §3, and the custody invariant and `docs/HUB.md` entry to `AGENTS.md`.
