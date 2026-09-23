// Per-agent-kind MCP client setup copy for the Preferences > MCP tab
// (`McpConnectAgentPanel.tsx`). Kept as data, separate from the component
// that renders it, since the branching content is the bulk of what grew
// this feature past a single generic panel.
//
// `claude-code`'s steps are the only ones verified against a real client:
// live-tested via `claude mcp add --transport http ... --scope user` and
// `claude mcp list` against a running Demeteo instance (see decision 46,
// docs/DECISIONS.md). The other four kinds have no confirmed MCP-client
// config anywhere in this repo or upstream docs Demeteo vendors, so their
// entries stay honest about that rather than inventing CLI syntax or config
// JSON this codebase can't verify (AGENTS.md's harness-truthfulness stance,
// docs/HARNESS_BASELINE.md).

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

export interface McpAgentSetup {
  /** `true` when these steps were verified against the real client. `false`
   *  renders the honest-degrade panel instead of a numbered walkthrough. */
  verified: boolean;
  steps: McpSetupStep[];
}

export const MCP_AGENT_SETUP: Record<string, McpAgentSetup> = {
  'claude-code': {
    verified: true,
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
  opencode: { verified: false, steps: [] },
  hermes: { verified: false, steps: [] },
  codex: { verified: false, steps: [] },
  pi: { verified: false, steps: [] },
};
