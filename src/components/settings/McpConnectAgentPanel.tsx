import { useState } from 'react';
import { save } from '@tauri-apps/plugin-dialog';
import {
  Boxes,
  Check,
  Circle,
  Copy,
  Download,
  MessageSquare,
  RotateCw,
  SquareCode,
  Terminal,
} from 'lucide-react';
import { AGENTS } from '../../lib/agents';
import { installMcpSkill } from '../../lib/mcpServer';
import { MCP_AGENT_SETUP, type McpAgentSetup, type McpSetupStep } from '../../lib/mcpAgentSetup';
import { reportError } from '../../lib/errorBus';
import { TabBar } from '../ui/TabBar';
import type { TabDef } from '../ui/TabBar';

/** Generic glyphs, not brand logos — no icon/asset system exists for agent
 *  kinds anywhere in the app (`src/lib/agents.ts` is stringly-typed), and
 *  sourcing real per-vendor marks is a separate, larger piece of work. */
const AGENT_ICONS: Record<string, React.ReactNode> = {
  'claude-code': <Terminal className="w-3.5 h-3.5" />,
  opencode: <SquareCode className="w-3.5 h-3.5" />,
  hermes: <MessageSquare className="w-3.5 h-3.5" />,
  codex: <Boxes className="w-3.5 h-3.5" />,
  pi: <Circle className="w-3.5 h-3.5" />,
};

const AGENT_KINDS = Object.keys(AGENTS);

function CopyableCommand({ command }: { command: string }) {
  const [copied, setCopied] = useState(false);
  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard access denied (e.g. devtools focus) — fail silently.
    }
  };
  return (
    <div className="flex items-center gap-2 mt-1.5">
      <code className="flex-1 font-mono text-xs text-cyan-300 bg-black/40 border border-white/5 rounded-lg px-3 py-2 break-all">
        {command}
      </code>
      <button
        type="button"
        onClick={handleCopy}
        aria-label="Copy command"
        className="shrink-0 p-2 rounded-lg bg-white/5 border border-white/10 hover:bg-violet-500/10 hover:border-violet-500/30 hover:text-violet-400 text-slate-400 transition-all"
      >
        {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
      </button>
    </div>
  );
}

function InstallSkillButton() {
  const [saving, setSaving] = useState(false);
  const handleInstall = async () => {
    setSaving(true);
    try {
      const destination = await save({ defaultPath: 'SKILL.md' });
      if (destination === null) return;
      await installMcpSkill(destination);
    } catch (e) {
      reportError(e);
    } finally {
      setSaving(false);
    }
  };
  return (
    <button
      type="button"
      onClick={handleInstall}
      disabled={saving}
      title="Save into .claude/skills/demeteo-mcp/ in your project"
      className="flex items-center gap-1.5 mt-1.5 px-3 py-1.5 text-xs font-medium rounded-lg bg-white/5 border border-white/10 hover:bg-violet-500/10 hover:border-violet-500/30 hover:text-violet-400 text-slate-300 transition-all disabled:opacity-50"
    >
      {saving ? <RotateCw className="w-3.5 h-3.5 animate-spin" /> : <Download className="w-3.5 h-3.5" />}
      Install skill
    </button>
  );
}

function SetupStepRow({ step, index, serverUrl }: { step: McpSetupStep; index: number; serverUrl: string }) {
  const command = step.command?.replace('{{url}}', serverUrl);
  return (
    <li className={`flex gap-3 ${step.optional ? 'opacity-70' : ''}`}>
      <span className="shrink-0 w-5 h-5 rounded-full bg-white/5 border border-white/10 text-[10px] font-mono text-slate-400 flex items-center justify-center mt-0.5">
        {index + 1}
      </span>
      <div className="flex-1 min-w-0">
        <p className="text-xs text-slate-300 leading-relaxed">
          {step.text}
          {step.optional && <span className="text-slate-500"> (optional)</span>}
        </p>
        {command && <CopyableCommand command={command} />}
        {step.action === 'install-skill' && <InstallSkillButton />}
      </div>
    </li>
  );
}

function UnverifiedAgentNotice({ label, serverUrl }: { label: string; serverUrl: string }) {
  return (
    <div className="space-y-3">
      <p className="text-xs text-slate-300 leading-relaxed">
        Give {label} this URL as a remote MCP server:
      </p>
      <CopyableCommand command={serverUrl} />
      <p className="text-xs text-slate-400 leading-relaxed">
        Approval happens inside this Demeteo window, which raises itself
        automatically — there is no token to paste into {label}'s own config.
      </p>
      <p className="text-xs text-amber-300/90 leading-relaxed">
        Demeteo doesn't have confirmed setup steps for {label} yet — check its
        own docs for configuring a remote MCP server, then point it at the URL
        above.
      </p>
    </div>
  );
}

function SetupSteps({ steps, serverUrl }: { steps: McpSetupStep[]; serverUrl: string }) {
  return (
    <ol className="space-y-3">
      {steps.map((step, index) => (
        <SetupStepRow key={step.text} step={step} index={index} serverUrl={serverUrl} />
      ))}
    </ol>
  );
}

function AgentSetup({ setup, label, serverUrl }: { setup: McpAgentSetup | undefined; label: string; serverUrl: string }) {
  switch (setup?.status) {
    case 'verified':
      return <SetupSteps steps={setup.steps} serverUrl={serverUrl} />;
    case 'unconfirmed':
      return (
        <div className="space-y-3">
          <p className="text-xs text-amber-300/90 leading-relaxed">
            Checked against Demeteo up to the approval prompt — a full {label} sign-in hasn't been
            confirmed yet.
          </p>
          <SetupSteps steps={setup.steps} serverUrl={serverUrl} />
        </div>
      );
    case 'incompatible':
      return (
        <div className="space-y-2">
          <p className="text-xs font-medium text-ruby-400">{label} can't connect to Demeteo yet.</p>
          <p className="text-xs text-slate-400 leading-relaxed">{setup.reason}</p>
        </div>
      );
    default:
      return <UnverifiedAgentNotice label={label} serverUrl={serverUrl} />;
  }
}

export function McpConnectAgentPanel({ serverUrl }: { serverUrl: string }) {
  const [selected, setSelected] = useState<string>('claude-code');
  const setup = MCP_AGENT_SETUP[selected];
  const label = AGENTS[selected]?.label ?? selected;

  const tabs: TabDef<string>[] = AGENT_KINDS.map((kind) => ({
    value: kind,
    label: AGENTS[kind]?.label ?? kind,
    icon: AGENT_ICONS[kind],
  }));

  return (
    <div className="nested-card p-4 space-y-3">
      <h4 className="font-heading text-xs font-semibold text-slate-300 uppercase tracking-wider">
        Connect an agent
      </h4>
      <TabBar tabs={tabs} activeTab={selected} onChange={setSelected} ariaLabel="Agent to set up" size="sm" />
      <div className="pt-1">
        <AgentSetup setup={setup} label={label} serverUrl={serverUrl} />
      </div>
    </div>
  );
}

export default McpConnectAgentPanel;
