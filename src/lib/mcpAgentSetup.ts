// Per-agent-kind MCP client setup copy for the Preferences > MCP tab
// (`McpConnectAgentPanel.tsx`). Kept as data, separate from the component
// that renders it, since the branching content is the bulk of what grew
// this feature past a single generic panel.
//
// How far each kind's steps have been checked is its `status`; the evidence
// lives with each entry. `unconfirmed` entries were run against a live
// Demeteo in an isolated config home up to — not through — the consent
// prompt. `incompatible` entries give no steps at all: handing a user a
// walkthrough that ends in a silent failure is worse than saying so
// (AGENTS.md's harness-truthfulness stance, docs/HARNESS_BASELINE.md).

export interface McpSetupStep {
  /** Plain instruction text. */
  text: string;
  /** A copy-pasteable command/URL shown under `text`, if any. `{{url}}` is
   *  replaced with the live server URL by the component. */
  command?: string;
  /** Marks a step as optional (rendered de-emphasized, e.g. "Install skill"). */
  optional?: boolean;
  /** Renders a real action button instead of a copy block. The only value
   *  today wires to the existing "Install skill" Tauri command. */
  action?: 'install-skill';
}

/** `verified`: a human completed the whole flow, consent included.
 *  `unconfirmed`: every step up to the consent prompt ran against a live
 *  Demeteo; rendered with a caveat. `incompatible`: the client cannot
 *  complete sign-in against this server; `reason` says why, no steps. */
export type McpSetupStatus = 'verified' | 'unconfirmed' | 'incompatible';

export interface McpAgentSetup {
  status: McpSetupStatus;
  steps: McpSetupStep[];
  /** Required for `incompatible`: what breaks, in the user's terms. */
  reason?: string;
}

/** Demeteo answers `initialize` and `tools/list` without a token and first
 *  says 401 on `tools/call` (docs/MCP_INTEGRATION.md §5). A client whose MCP
 *  SDK starts OAuth only on a connect-time 401 therefore never starts it.
 *  Claude Code and Codex start it from the `.well-known` metadata instead,
 *  and Pi's adapter on `"auth": "oauth"`. */
function noConnectTimeChallenge(label: string): string {
  return `Demeteo lets clients connect and list tools without signing in, and asks for sign-in only when a tool is called. ${label} starts sign-in only when connecting is refused, so it never gets a token: it shows Demeteo as connected, and every tool call then fails as unauthorized.`;
}

export const MCP_AGENT_SETUP: Record<string, McpAgentSetup> = {
  // Live-tested end to end against a running Demeteo (decision 46,
  // docs/DECISIONS.md).
  'claude-code': {
    status: 'verified',
    steps: [
      {
        text: 'Register Demeteo as a remote MCP server:',
        command: 'claude mcp add --transport http demeteo {{url}} --scope user',
      },
      {
        text: 'Check it registered — this will show "Needs authentication":',
        command: 'claude mcp list',
      },
      {
        text: 'Start a Claude Code session, run /mcp, select demeteo, then choose Authenticate. The approval prompt appears inside this Demeteo window — it raises itself automatically. A browser tab may also open and sit there loading; that\'s expected, it completes once you approve here, not in the browser.',
        command: '/mcp',
      },
      {
        text: 'Confirm it connected:',
        command: 'claude mcp list',
      },
      {
        text: 'Install the skill that teaches Claude Code which of these tools to use for a given question, and which never to call speculatively.',
        optional: true,
        action: 'install-skill',
      },
    ],
  },
  // codex-cli 0.155.1: `mcp list` reports "Not logged in" (OAuth detected,
  // not "Unsupported"). `mcp add --url` starts sign-in itself on detecting
  // OAuth, which is why step 1 carries the consent copy.
  codex: {
    status: 'unconfirmed',
    steps: [
      {
        text: 'Register Demeteo as a remote MCP server. Codex detects that it needs sign-in and starts it straight away — approve the prompt that appears in this Demeteo window. A browser tab may sit loading; it completes once you approve here.',
        command: 'codex mcp add demeteo --url {{url}}',
      },
      {
        text: 'If sign-in did not finish, run it again:',
        command: 'codex mcp login demeteo',
      },
      {
        text: 'Confirm it connected — the Auth column should say OAuth, not "Not logged in":',
        command: 'codex mcp list',
      },
      {
        text: 'Start a new Codex session and run /mcp to see Demeteo\'s tools.',
        command: '/mcp',
      },
    ],
  },
  // pi 0.85.1 has no MCP by design; pi-mcp-adapter supplies it. `"auth":
  // "oauth"` is load-bearing: without it the adapter signs in only on a
  // connect-time 401, which Demeteo never sends (see noConnectTimeChallenge).
  // It reports "connected" before sign-in, so step 3 is not optional.
  pi: {
    status: 'unconfirmed',
    steps: [
      {
        text: 'Pi has no built-in MCP client. Install the pi-mcp-adapter extension, then restart Pi:',
        command: 'pi install npm:pi-mcp-adapter',
      },
      {
        text: 'Add Demeteo to ~/.pi/agent/mcp.json (create it, or merge into an existing mcpServers object):',
        command: '{ "mcpServers": { "demeteo": { "url": "{{url}}", "auth": "oauth" } } }',
      },
      {
        text: 'In a Pi session, sign in — Pi lists Demeteo as connected before this, but tool calls fail until it is done. Approve the prompt in this Demeteo window; a browser tab may also open and completes once you do.',
        command: '/mcp-auth demeteo',
      },
      {
        text: 'Confirm Demeteo\'s tools are listed:',
        command: '/mcp tools',
      },
    ],
  },
  // opencode 1.18.7: `mcp list` says "connected", and `mcp auth` prints
  // "Authentication successful!" having stored no token.
  opencode: {
    status: 'incompatible',
    steps: [],
    reason: noConnectTimeChallenge('OpenCode'),
  },
  // Source-read only (hermes-agent via mcp==2.0.0); not installed where this
  // was researched. `hermes mcp login` warns that no token was obtained.
  hermes: {
    status: 'incompatible',
    steps: [],
    reason: noConnectTimeChallenge('Hermes'),
  },
};
