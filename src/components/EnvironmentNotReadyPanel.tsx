import { AlertTriangle, Terminal, Wrench } from 'lucide-react';
import type { EnvironmentFailure } from '../lib/harnessVerdict';

// HB7 — a terminal environment failure, presented as what it is.
//
// The engine already knows this class of failure apart from every other one: it
// fires `EnvironmentNotReady`, terminates without spending the retry budget,
// and composes a message whose entire payload is the remediation. The UI used
// to render that message as a ruby monospace blob under "Step failed. You can
// change harness/model and retry." — which is wrong twice over. It is not the
// feature's defect, and retrying is the one thing that cannot help: no edit to
// the source installs a system library.
//
// So the remediation is the **body**, in prose, first. The reason, the failing
// command, the machine and the reproduce line are the supporting evidence
// underneath it. Amber rather than ruby, matching the accent `NotificationBell`
// already gives `environment_not_ready` and reserving ruby (AGENTS.md §4) for
// failures the feature actually caused.

/**
 * Why an environment failure happens on a machine where the tool is, as far as
 * the user is concerned, already installed — and what "installed" has to mean
 * for the harness.
 *
 * Deliberately names no specific tool: the harness runs whatever prepare/test
 * command the project declares, so the rule is the same for cargo, npm, pytest
 * or anything else. The trap it describes is the common one — the tool exists,
 * but only the user's own shell can see it, because a version manager activates
 * it per-directory (or only from a config the harness's shell never reads).
 */
function ShellHint() {
  return (
    <div
      className="mt-3 flex items-start gap-2 border-t border-amber-500/20 pt-3 font-sans text-slate-400"
      title={
        'The harness runs your prepare/test commands through a fresh interactive login shell on the ' +
        'target machine (bash -l -i -c), not through your terminal session. A tool is only usable if ' +
        'that shell can find it on PATH.\n\n' +
        'Verify on the machine:\n' +
        "  bash -l -i -c 'command -v <tool>'\n\n" +
        'If it prints nothing, export the tool from ~/.profile or ~/.bashrc, or declare it in your ' +
        "version manager's global config (mise use -g, asdf global, nvm alias default) so every " +
        'shell activates it — not just the directories that ask for it.'
      }
    >
      <AlertTriangle className="mt-px w-3.5 h-3.5 shrink-0 text-amber-400" />
      <span className="text-[11px] leading-relaxed">
        The harness runs commands through a fresh interactive login shell, so every tool must be
        discoverable on that shell&apos;s <span className="font-mono text-slate-300">PATH</span> —
        installed is not enough. Hover for how to check and fix it.
      </span>
    </div>
  );
}

interface Props {
  failure: EnvironmentFailure;
  /**
   * True when the run's own baseline already recorded this gate as unrunnable
   * — HB9's halt at the head of the graph. Worth saying, because it changes
   * what the user is looking at: the run stopped before a single agent turn,
   * so nothing was implemented and nothing was spent on it.
   */
  atBase: boolean;
}

/**
 * The remediation-first presentation of a terminal environment failure.
 *
 * Renders the same information the raw `error_message` carried, in the order
 * that makes it actionable: what to do, then why, then how to reproduce.
 */
export function EnvironmentNotReadyPanel({ failure, atBase }: Props) {
  return (
    <section
      data-testid="environment-not-ready"
      className="glass-panel mt-3 border border-amber-500/25 p-4"
    >
      <header className="flex items-start gap-2.5">
        <Wrench className="mt-0.5 w-4 h-4 shrink-0 text-amber-400" />
        <div>
          <h4 className="font-heading text-sm font-semibold tracking-wide text-amber-300">
            Environment not ready — the machine, not the feature
          </h4>
          <p className="mt-1 font-sans text-[11px] leading-relaxed text-slate-400">
            {atBase
              ? 'This gate was already failing at the base commit because this machine cannot run it, so the run stopped at the baseline — before any implementation budget was spent. It proved nothing about the change.'
              : 'The command never produced a result here, so it says nothing about the change. Editing the code cannot fix it and retrying will fail the same way until the machine is provisioned.'}
          </p>
        </div>
      </header>

      {failure.remediation === '' ? (
        <p
          data-testid="environment-no-remediation"
          className="mt-4 font-sans text-[13px] leading-relaxed text-slate-300"
        >
          No remediation was suggested for this failure. The reason below is everything the
          orchestrator could establish — reproduce it with the command at the bottom.
        </p>
      ) : (
        <div className="mt-4 rounded-lg border border-amber-500/20 bg-amber-500/[0.06] p-3.5">
          <div className="font-sans text-[10px] font-bold uppercase tracking-wider text-amber-400/80">
            Do this
          </div>
          <p
            data-testid="environment-remediation"
            className="mt-1.5 whitespace-pre-wrap font-sans text-[13px] leading-relaxed text-slate-100"
          >
            {failure.remediation}
          </p>
        </div>
      )}

      {failure.reason !== '' && (
        <p className="mt-3 whitespace-pre-wrap font-sans text-xs leading-relaxed text-slate-400">
          {failure.reason}
        </p>
      )}

      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[11px]">
        <dt className="text-slate-500">command</dt>
        <dd className="break-all text-slate-300">{failure.command}</dd>
        <dt className="text-slate-500">machine</dt>
        <dd className="break-all text-slate-300">{failure.machine || 'local'}</dd>
      </dl>

      {failure.reproduce !== '' && (
        <div className="mt-3">
          <div className="flex items-center gap-1.5 font-sans text-[10px] font-bold uppercase tracking-wider text-slate-500">
            <Terminal className="w-3 h-3" /> Reproduce
          </div>
          <pre className="mt-1.5 overflow-x-auto rounded border border-white/10 bg-black/30 p-2.5 font-mono text-[11px] leading-relaxed text-cyan-300">
            {failure.reproduce}
          </pre>
        </div>
      )}

      <ShellHint />
    </section>
  );
}
