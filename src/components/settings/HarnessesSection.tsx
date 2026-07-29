import { AlertTriangle, Check, ChevronDown, ChevronUp, Plus, RotateCw, Trash2, Zap } from 'lucide-react';
import { useSettings } from './ProjectSettingsContext';
import type { CommandProbeReport, ProbedCommand, ProbedCommandSource } from '../../lib/project';

// The Strategy tab's harness half, extracted (HB6): each row now carries five
// columns — name, command, gate checkbox, order, probe status — and both
// `StrategyTab` and `ProjectSettingsContext` were already past the ~400 LOC
// convention before it grew any of them.
//
// Deliberately *project*-scoped and not a global settings surface: harness
// commands are repo-specific by nature (`npm run lint` vs `cargo clippy` vs
// `ruff check`), so a global map would be wrong for nearly every project and
// would invent a global-vs-project shadowing question for no benefit. The
// reusable knowledge is the ecosystem recipe, which belongs in detection.

/** The probe answer for one configured command, if the machine answered. */
function probeFor(
  report: CommandProbeReport | null,
  source: ProbedCommandSource,
  harness?: string,
): ProbedCommand | undefined {
  return report?.commands.find(
    c => c.source === source && (harness === undefined || c.harness === harness),
  );
}

/**
 * Whether the machine could find the binaries one command names.
 *
 * Emerald resolved / ruby missing, per AGENTS.md §4. A command whose binaries
 * the engine deliberately skipped (a builtin, a `$(…)` substitution, a glob)
 * renders as *unchecked* rather than healthy: the probe asserts only what it
 * actually asked, and claiming more is how an indicator starts lying.
 */
function ProbeStatus({ probe, probing }: { probe?: ProbedCommand; probing: boolean }) {
  if (!probe) {
    return (
      <span className="text-[10px] font-mono text-slate-600 whitespace-nowrap">
        {probing ? 'checking…' : 'not checked'}
      </span>
    );
  }
  if (probe.binaries.length === 0) {
    return (
      <span className="text-[10px] font-mono text-slate-500 whitespace-nowrap">
        nothing to check
      </span>
    );
  }
  return (
    <span className="flex flex-wrap gap-1">
      {probe.binaries.map(b => (
        <span
          key={b.name}
          className={`flex items-center gap-1 px-1.5 py-0.5 text-[9px] rounded font-mono border ${
            b.resolved
              ? 'bg-emerald-500/10 border-emerald-500/20 text-emerald-400'
              : 'bg-ruby-500/10 border-ruby-500/20 text-ruby-400'
          }`}
        >
          {b.resolved ? <Check className="w-2.5 h-2.5 stroke-[3]" /> : <AlertTriangle className="w-2.5 h-2.5" />}
          <span>{b.name}</span>
          <span className="opacity-70">{b.resolved ? 'resolved' : 'missing'}</span>
        </span>
      ))}
    </span>
  );
}

export function HarnessesSection() {
  const s = useSettings();

  const machine = s.commandProbe?.machine ?? '';
  const machineLabel =
    machine === '' ? '' : machine === 'local'
      ? 'this computer'
      : (s.machines.find(m => m.id === machine)?.name ?? machine);

  // Gates first, in the order they will run; then the rest, alphabetically.
  // Order is stored only for the selection because that is the only place it
  // means anything — an unticked harness never runs, so it has no position.
  const gates = s.validationGates.filter(g =>
    Object.prototype.hasOwnProperty.call(s.harnesses, g),
  );
  const isGate = (name: string) => gates.includes(name);
  const rows = [
    ...gates,
    ...Object.keys(s.harnesses).filter(n => !isGate(n)).sort((a, b) => a.localeCompare(b)),
  ];

  const toggleGate = (name: string) =>
    s.setValidationGates(isGate(name) ? gates.filter(g => g !== name) : [...gates, name]);

  const moveGate = (name: string, delta: number) => {
    const i = gates.indexOf(name);
    const j = i + delta;
    if (i < 0 || j < 0 || j >= gates.length) return;
    const next = [...gates];
    [next[i], next[j]] = [next[j], next[i]];
    s.setValidationGates(next);
  };

  const deleteHarness = (name: string) => {
    const copy = { ...s.harnesses };
    delete copy[name];
    s.setHarnesses(copy);
    s.setValidationGates(s.validationGates.filter(g => g !== name));
  };

  return (
    <div className="glass-panel p-6 rounded-xl md:col-span-2 space-y-4">
      <div className="flex items-center justify-between border-b border-white/5 pb-2">
        <h3 className="font-outfit text-sm font-semibold text-slate-300 uppercase tracking-wider flex items-center gap-2">
          <Zap className="w-4 h-4 text-cyan-400" /> Validation Harnesses
        </h3>
        <div className="flex items-center gap-3">
          {/* Which machine was asked. Not decoration: on a remote-compute
              project the commands run there, not here, so an indicator that
              doesn't say where it looked is a lie on exactly those projects. */}
          <span className="text-[11px] font-mono text-slate-500">
            {s.probeError
              ? 'commands not checked'
              : machineLabel
                ? `checked on ${machineLabel}`
                : 'not checked yet'}
          </span>
          <button
            type="button"
            onClick={s.refreshCommandProbe}
            disabled={s.isProbingCommands}
            className="p-1 rounded text-slate-500 hover:text-cyan-400 hover:bg-white/5 transition-all disabled:opacity-50"
            aria-label="Re-check commands"
          >
            <RotateCw className={`w-3.5 h-3.5 ${s.isProbingCommands ? 'animate-spin text-cyan-400' : ''}`} />
          </button>
        </div>
      </div>

      <p className="text-xs text-slate-400 leading-relaxed">
        The commands that judge a feature, run on the project's own machine through an interactive
        login shell (<code className="text-slate-300">bash -l -i -c</code>) — the same shell you get
        when you open a terminal, so <code className="text-slate-300">mise</code>/
        <code className="text-slate-300">asdf</code>/<code className="text-slate-300">nvm</code>{' '}
        shims resolve. Tick the ones that should gate validation; they run in the order below.
      </p>

      {/* The engine's own sentence about what a harness command has to survive
          — rendered rather than paraphrased, so this and the failure a user
          meets mid-run cannot drift apart. */}
      {s.commandProbe && (
        <p className="text-[11px] text-slate-500 leading-relaxed border-l-2 border-white/10 pl-3">
          {s.commandProbe.guidance}
        </p>
      )}

      <div>
        <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider" htmlFor="prepare-command">Prepare Command (optional)</label>
        <div className="flex gap-2 items-center">
          <input id="prepare-command" type="text" value={s.prepareCommand} onChange={e => s.setPrepareCommand(e.target.value)} placeholder="e.g. npm ci or cargo fetch" className="flex-1 min-w-0 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600" />
          <ProbeStatus probe={probeFor(s.commandProbe, 'prepare')} probing={s.isProbingCommands} />
        </div>
        <p className="text-[11px] text-slate-500 mt-1.5 leading-relaxed">
          Runs inside each subtask worktree right before the harness. Demeteo already symlinks gitignored dependency
          caches (<code className="text-slate-300">node_modules/</code>, <code className="text-slate-300">target/</code>, <code className="text-slate-300">.venv/</code>, …) in
          from the primary checkout, so most projects don't need this — reach for it only for codegen, DB migrations,
          or a freshly-added dependency the symlink can't cover.
        </p>
      </div>

      <div>
        <label className="block text-xs font-mono text-slate-400 mb-1.5 uppercase tracking-wider" htmlFor="test-command">Default Test Command</label>
        <div className="flex gap-2 items-center">
          <input id="test-command" type="text" value={s.testCommand} onChange={e => s.setTestCommand(e.target.value)} placeholder="e.g. npm test or cargo test" className="flex-1 min-w-0 bg-black/40 border border-white/10 rounded-lg py-2 px-3 text-sm text-white focus:outline-none focus:border-cyan-500/50 font-mono placeholder-slate-600" />
          <ProbeStatus probe={probeFor(s.commandProbe, 'test')} probing={s.isProbingCommands} />
        </div>
        <p className="text-[11px] text-slate-500 mt-1.5 leading-relaxed">
          The fallback gate: it runs when a step names no harness of its own and no harness below is ticked.
        </p>
      </div>

      <div className="space-y-3">
        <div className="flex items-center justify-between text-[10px] font-mono text-slate-500 uppercase tracking-wider border-t border-white/5 pt-3">
          <span>Named harnesses</span>
          <span>{gates.length > 0 ? `${gates.length} gating validation` : 'none gating — falls back to the test command'}</span>
        </div>
        {rows.map((name) => {
          const gating = isGate(name);
          const position = gates.indexOf(name);
          return (
            <div key={name} className={`flex gap-2 items-center p-2 rounded-lg border transition-all ${gating ? 'bg-emerald-500/5 border-emerald-500/20' : 'bg-black/20 border-white/5'}`}>
              <label className="flex items-center gap-2 shrink-0 cursor-pointer">
                <input
                  type="checkbox"
                  checked={gating}
                  onChange={() => toggleGate(name)}
                  aria-label={`${name} gates validation`}
                  className="w-4 h-4 rounded border-white/20 bg-black/40 text-emerald-500 focus:ring-emerald-500/40 focus:ring-offset-0"
                />
                <span className="text-[10px] font-mono text-slate-500 uppercase tracking-wider hidden md:inline">gates</span>
              </label>
              <div className="flex-1 min-w-0 font-mono text-xs text-white truncate">
                <span className="text-cyan-400">{name}</span>: <span className="text-slate-300">{s.harnesses[name]}</span>
              </div>
              <ProbeStatus probe={probeFor(s.commandProbe, 'harness', name)} probing={s.isProbingCommands} />
              {/* Order is the user's — cheap gates first, lint before
                  integration — and only a gate has one, since a harness that
                  never runs has no position in the run. */}
              <div className="flex flex-col shrink-0">
                <button type="button" onClick={() => moveGate(name, -1)} disabled={!gating || position <= 0} aria-label={`Run ${name} earlier`} className="p-0.5 text-slate-500 hover:text-cyan-400 disabled:opacity-20 disabled:hover:text-slate-500">
                  <ChevronUp className="w-3.5 h-3.5" />
                </button>
                <button type="button" onClick={() => moveGate(name, 1)} disabled={!gating || position < 0 || position >= gates.length - 1} aria-label={`Run ${name} later`} className="p-0.5 text-slate-500 hover:text-cyan-400 disabled:opacity-20 disabled:hover:text-slate-500">
                  <ChevronDown className="w-3.5 h-3.5" />
                </button>
              </div>
              <button type="button" onClick={() => deleteHarness(name)} className="p-2 text-slate-500 hover:text-ruby-400 bg-white/5 rounded-lg border border-white/5 hover:bg-white/10 shrink-0" aria-label={`Delete ${name} harness`}>
                <Trash2 className="w-3.5 h-3.5" />
              </button>
            </div>
          );
        })}
        <div className="border-t border-white/5 pt-3 flex gap-2">
          <input type="text" placeholder="Name" id="new-harness-name" aria-label="New harness name" className="w-1/3 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono" />
          <input type="text" placeholder="Command" id="new-harness-cmd" aria-label="New harness command" className="flex-1 bg-black/40 border border-white/10 rounded-lg py-1.5 px-3 text-xs text-white placeholder-slate-600 focus:outline-none focus:border-cyan-500/50 font-mono" />
          <button type="button" onClick={() => {
            const nameEl = document.getElementById('new-harness-name') as HTMLInputElement;
            const cmdEl = document.getElementById('new-harness-cmd') as HTMLInputElement;
            if (nameEl && cmdEl) {
              const name = nameEl.value.trim(); const cmd = cmdEl.value.trim();
              if (name && cmd) { s.setHarnesses({ ...s.harnesses, [name]: cmd }); nameEl.value = ''; cmdEl.value = ''; }
            }
          }} aria-label="Add harness" className="px-3 py-1.5 text-xs bg-cyan-600 hover:bg-cyan-500 text-white rounded-lg transition-colors flex items-center gap-1 font-semibold shrink-0">
            <Plus className="w-3 h-3" /> Add
          </button>
        </div>
      </div>

      {/* The engine's own launch-blocking message, verbatim — it carries the
          missing binary and the `bash -l -i -c` reproduce line. A parallel copy
          here would drift out of agreement with what a blocked launch says. */}
      {s.commandProbe?.detail && (
        <div className={`rounded-lg p-3 border text-[11px] leading-relaxed whitespace-pre-wrap font-mono ${s.commandProbe.blocks_launch ? 'bg-ruby-500/10 border-ruby-500/20 text-ruby-200' : 'bg-black/30 border-white/5 text-slate-400'}`}>
          {s.commandProbe.detail}
        </div>
      )}

      {/* A probe that could not run is an indicator that failed, not a
          configuration verdict: the machine may simply be off. Saving is
          untouched by it — the gate that matters stays at launch, where which
          machine will run the commands is known. */}
      {s.probeError && (
        <div className="rounded-lg p-3 border border-white/10 bg-black/30 text-[11px] leading-relaxed text-slate-400">
          These commands could not be checked right now ({s.probeError}). That does not stop you
          saving them — they are checked again at launch, on the machine that will run them.
        </div>
      )}
    </div>
  );
}
