// Three dimensions and not one verdict, because the harness that decides the
// shape — codex — refuses one class of access and serves another, and a note
// that reports either one alone is a false sentence about the user's other
// repositories. So each dimension is asserted on its own text and its own
// weight, and "nobody has said" still renders nothing at all.

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { HarnessContainmentNote } from './HarnessContainmentNote';
import type { AgentCatalogEntry } from '../../lib/agentCatalog';
import type { PathContainment } from '../../lib/pathContainment';

const CATALOG: AgentCatalogEntry[] = [
  { kind: 'codex', display_label: 'Codex', lists_models: false, default_model: null, install_command: '' },
  { kind: 'opencode', display_label: 'OpenCode', lists_models: false, default_model: null, install_command: '' },
  { kind: 'claude-code', display_label: 'Claude Code', lists_models: false, default_model: null, install_command: '' },
];

const CODEX: PathContainment = { reads: 'none', writes: 'os', shell: 'os' };
const OPENCODE: PathContainment = { reads: 'harness', writes: 'harness', shell: 'harness-partial' };
const UNFENCED: PathContainment = { reads: 'none', writes: 'none', shell: 'none' };
/** No harness declares this today; it is what a note with nothing to warn
 *  about has to look like, and the only fixture that proves the weight is
 *  driven by the answer rather than always on. */
const SEALED: PathContainment = { reads: 'harness', writes: 'harness', shell: 'harness' };

function rows(kind: string, path_containment?: PathContainment) {
  return [{ kind, path_containment }];
}

function mount(kind: string, containment?: PathContainment) {
  return render(
    <HarnessContainmentNote agents={CATALOG} machineAgents={rows(kind, containment)} kind={kind} />,
  );
}

const line = (dimension: string) =>
  screen.getByTestId('harness-containment').querySelector(`[data-dimension="${dimension}"]`)!;

describe('HarnessContainmentNote', () => {
  it('says a kernel write fence is not a read fence, on the harness where that is the whole story', () => {
    mount('codex', CODEX);

    expect(line('reads')).toHaveAttribute('data-enforcement', 'none');
    expect(line('reads')).toHaveTextContent('nothing stops Codex reading any file your account can');
    expect(line('reads')).toHaveTextContent('your other repositories');
    expect(line('writes')).toHaveAttribute('data-enforcement', 'os');
    expect(line('writes')).toHaveTextContent('the kernel refuses a write');
    // The sandbox's writable roots are wider than the worktree, so the note may
    // not shrink them to it — and Demeteo never pins them, so it may not close
    // the list either: the harness's own config file, which Demeteo is
    // forbidden to read, is free to add a root.
    expect(line('writes')).toHaveTextContent('temporary directories');
    expect(line('writes')).toHaveTextContent("plus whatever Codex's own config adds");
    expect(line('shell')).toHaveTextContent('the kernel refuses, not the agent');
  });

  it('weights the dimension nothing refuses and leaves the fenced ones as furniture', () => {
    mount('codex', CODEX);

    expect(line('reads')).toHaveClass('text-amber-200/90');
    expect(line('writes')).toHaveClass('text-slate-500');
    expect(line('shell')).toHaveClass('text-slate-500');
    // Nothing has failed and the run is launchable; ruby would say the
    // opposite, and `role="alert"` would interrupt for a standing fact.
    const note = screen.getByTestId('harness-containment');
    expect(note.className).not.toContain('ruby');
    expect(note).not.toHaveAttribute('role', 'alert');
  });

  it('does not let a half-covered shell read as a covered one', () => {
    mount('opencode', OPENCODE);

    expect(line('reads')).toHaveClass('text-slate-500');
    // The rule is the file tools' and the shell line below says so; a reader
    // who stops at the line named for the access they care about must not come
    // away with an absolute.
    expect(line('reads')).toHaveTextContent(
      "OpenCode's own file tools refuse to open a file outside this worktree",
    );
    expect(line('shell')).toHaveAttribute('data-enforcement', 'harness-partial');
    // The distinction the arm exists for: copy that stopped at "the rule
    // applies" would describe the gap as covered.
    expect(line('shell')).toHaveTextContent('only part of what it runs through a shell');
    expect(line('shell')).toHaveTextContent('not checked against anything');
    expect(line('shell')).toHaveClass('text-amber-200/90');
  });

  it('gives an unfenced harness every line and an action the screen can perform', () => {
    mount('claude-code', UNFENCED);

    for (const dimension of ['reads', 'writes', 'shell']) {
      expect(line(dimension)).toHaveAttribute('data-enforcement', 'none');
      expect(line(dimension)).toHaveClass('text-amber-200/90');
    }
    // The harness picker is the control directly above this note.
    expect(screen.getByTestId('harness-containment')).toHaveTextContent(
      'Choose another harness above',
    );
  });

  it('drops the panel and the action when every dimension is refused by something', () => {
    mount('opencode', SEALED);

    const note = screen.getByTestId('harness-containment');
    expect(note.className).not.toContain('amber');
    expect(note).not.toHaveTextContent('Choose another harness above');
    expect(line('shell')).toHaveClass('text-slate-500');
  });

  it('renders nothing until the machine has actually answered', () => {
    const { rerender } = render(
      <HarnessContainmentNote agents={CATALOG} machineAgents={[]} kind="codex" />,
    );
    expect(screen.queryByTestId('harness-containment')).not.toBeInTheDocument();

    // No harness chosen yet.
    rerender(<HarnessContainmentNote agents={CATALOG} machineAgents={rows('codex', CODEX)} kind="" />);
    expect(screen.queryByTestId('harness-containment')).not.toBeInTheDocument();

    // A backend that predates the field. Absent is not "unfenced", and it is
    // certainly not "fenced".
    rerender(
      <HarnessContainmentNote agents={CATALOG} machineAgents={rows('codex')} kind="codex" />,
    );
    expect(screen.queryByTestId('harness-containment')).not.toBeInTheDocument();
  });

  // The Rust `PathContainment` is a wire contract either side can rename, and
  // its Tauri-side test says a rename should cost this note its rendering and
  // nothing more. Without the guard an unknown spelling reaches the copy table
  // as a missing key and takes the sync pane down mid-render.
  it('renders nothing, rather than throwing, for a spelling it does not know', () => {
    const alien = { ...OPENCODE, shell: 'harness_partial' } as unknown as PathContainment;
    mount('opencode', alien);
    expect(screen.queryByTestId('harness-containment')).not.toBeInTheDocument();
  });
});
