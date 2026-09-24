// Per-agent-kind MCP client setup copy for the Preferences > MCP tab
// (`McpConnectAgentPanel.tsx`). Kept as data, separate from the component
// that renders it, since the branching content is the bulk of what grew
// this feature past a single generic panel.
//
// How far each kind's steps have been checked is its `status`, and the
// evidence lives with each entry — never claim more than that evidence
// carries (AGENTS.md's harness-truthfulness stance, docs/HARNESS_BASELINE.md).

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
 *  `unconfirmed`: rendered under a caveat — by default that the steps ran
 *  against a live Demeteo in an isolated config home up to, not through,
 *  the consent prompt; `caveat` replaces it when the evidence is less. */
export type McpSetupStatus = 'verified' | 'unconfirmed';

export interface McpAgentSetup {
  status: McpSetupStatus;
  steps: McpSetupStep[];
  caveat?: string;
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
  // pi 0.85.1 has no MCP by design; pi-mcp-adapter supplies it, and
  // `"auth": "oauth"` makes it sign in instead of waiting to be refused.
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
        text: 'In a Pi session, sign in. Approve the prompt that appears in this Demeteo window; a browser tab may also open and completes once you do.',
        command: '/mcp-auth demeteo',
      },
      {
        text: 'Confirm Demeteo\'s tools are listed:',
        command: '/mcp tools',
      },
    ],
  },
  // opencode 1.18.7, against a mock that refuses an unauthenticated
  // `initialize` as Demeteo does: `mcp list` reported "needs authentication"
  // and `mcp auth` registered, then opened /authorize.
  opencode: {
    status: 'unconfirmed',
    steps: [
      {
        text: 'Add Demeteo to OpenCode\'s global config:',
        command: 'opencode mcp add demeteo --url {{url}}',
      },
      {
        text: 'Check it registered — this will show "needs authentication":',
        command: 'opencode mcp list',
      },
      {
        text: 'Sign in. Approve the prompt that appears in this Demeteo window; a browser tab may also open and completes once you do. OpenCode listens on port 19876 for the callback — if that port is taken, set oauth.callbackPort on the demeteo entry in its config.',
        command: 'opencode mcp auth demeteo',
      },
      {
        text: 'Confirm it signed in:',
        command: 'opencode mcp auth list',
      },
    ],
  },
  // Read from hermes-agent's source (mcp==2.0.0), never run: it signs in on
  // a connect-time 401 through the SDK's OAuthClientProvider.
  hermes: {
    status: 'unconfirmed',
    caveat:
      'Taken from Hermes\'s own source, not yet run against Demeteo — a full Hermes sign-in hasn\'t been confirmed.',
    steps: [
      {
        text: 'Add Demeteo as a sign-in protected server. Hermes connects and asks which tools to enable:',
        command: 'hermes mcp add demeteo --url {{url}} --auth oauth',
      },
      {
        text: 'Sign in from a terminal. Approve the prompt that appears in this Demeteo window; a browser tab may also open and completes once you do.',
        command: 'hermes mcp login demeteo',
      },
      {
        text: 'Check the connection and list the tools:',
        command: 'hermes mcp test demeteo',
      },
      {
        text: 'Start a new Hermes session, or run /reload-mcp in an open one.',
        command: '/reload-mcp',
      },
    ],
  },
};
