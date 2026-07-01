/**
 * Gate feedback loop — re-render of markdown viewer on every keystroke.
 *
 * Bug:
 *   Typing into the "Redirect / Loop" feedback `<textarea>` inside
 *   `src/components/GateView.tsx` re-renders the entire modal on every
 *   keystroke, including the heavy `<ReactMarkdown>` (and its embedded
 *   Monaco `<Editor>` blocks) inside `<ArtifactViewer>` mounted directly
 *   above the textarea. The user perceives this as "the md view refreshing
 *   every time I type a key".
 *
 * What this test asserts:
 *   In a corrected version of the bug, the heavy markdown viewer must NOT
 *   re-render when only the feedback text changes. With the bug in place,
 *   the heavy viewer re-renders once per keystroke (1 initial + N for N
 *   keystrokes). This script counts renders and exits non-zero while the
 *   bug is present.
 *
 * Run (no project deps installed — uses the sibling `demeteo/` worktree's
 * installed `react` and `react-test-renderer`):
 *
 *   $ cd tests/repro
 *   $ DEMETEO_REPO_PATH=../../../demeteo node gate-feedback-rerender.mjs
 *
 * The script:
 *   1. Boots React + react-test-renderer from $DEMETEO_REPO_PATH/node_modules.
 *   2. Mounts a `BuggyGateView`-shaped component that owns (a) a textarea
 *      whose value lives in `feedback` state and (b) a `HeavyArtifactViewer`
 *      child (a stand-in for the real ArtifactViewer, instrumented with a
 *      render counter).
 *   3. Simulates the user typing "hello" (5 chars).
 *   4. Asserts the heavy child rendered exactly 1 time.
 *
 * Expected:
 *   - Bug present (current `GateView.tsx`):  FAIL — viewer renders 6 times.
 *   - Bug fixed (e.g., `React.memo` on viewer + memoised props): PASS.
 */

import { createRequire } from 'node:module';
import { existsSync } from 'node:fs';
import path from 'node:path';

// Tell react-test-renderer that act() is supported, suppressing the
// "current testing environment is not configured to support act" warning.
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const here = path.dirname(new URL(import.meta.url).pathname);
const defaultRepo = path.resolve(here, '..', '..'); // tests/repro -> repo root
const repoPath = process.env.DEMETEO_REPO_PATH || defaultRepo;
const nmAbs = path.resolve(repoPath, 'node_modules');
if (!existsSync(path.join(nmAbs, 'react', 'index.js'))) {
  console.error(
    `[repro] Cannot find React under ${nmAbs}.\n` +
    `         Set DEMETEO_REPO_PATH to the demeteo worktree whose\n` +
    `         'npm install' has been run.`,
  );
  process.exit(2);
}
const require = createRequire(import.meta.url);
const React = require(path.join(nmAbs, 'react', 'index.js'));
const ReactTestRenderer = require(path.join(nmAbs, 'react-test-renderer', 'index.js'));

const { useState, useEffect, memo } = React;
const { create, act } = ReactTestRenderer;

/**
 * Stand-in for the heavy `<ArtifactViewer>` rendered above the textarea
 * inside GateView. In the real component, the render output is roughly:
 *
 *   <ReactMarkdown ... components={{ code: ({...}) => <Editor ... /> }}>
 *     {content}
 *   </ReactMarkdown>
 *
 * Each render walks the markdown AST and rebuilds (and feeds updated
 * props to) every embedded Monaco editor — see `src/components/ArtifactViewer.tsx:186-355`.
 *
 * `renderCount` is the only thing this test inspects.
 */
let renderCount = 0;
function HeavyArtifactViewer(_props) {
  renderCount += 1;
  return React.createElement('div', { 'data-testid': 'heavy-viewer' }, 'markdown body');
}

/**
 * Faithful shape of `src/components/GateView.tsx`:
 * the parent component owns `feedback` state for the textarea AND
 * renders the heavy artifact viewer in the same tree, with a stable
 * `artifactPath` prop. When `feedback` changes via setFeedback, the
 * parent re-renders, which (without React.memo) re-renders the heavy
 * viewer even though its inputs are unchanged.
 *
 * This is the exact pattern that produces "the md view refreshing on
 * every keystroke" in the real app.
 */
function BuggyGateView() {
  const [feedback, setFeedback] = useState('');
  const [stepExec] = useState({ artifact_paths: ['/tmp/research-report.md'], artifact_path: null });

  const gatePath = stepExec.artifact_paths.length
    ? stepExec.artifact_paths[0]
    : stepExec.artifact_path;

  return React.createElement(
    'div',
    null,
    React.createElement(HeavyArtifactViewer, { artifactPath: gatePath }),
    React.createElement('textarea', {
      'data-testid': 'feedback',
      value: feedback,
      onChange: (e) => setFeedback(e.target.value),
    }),
  );
}

function main() {
  renderCount = 0;
  let tree;
  act(() => {
    tree = create(React.createElement(BuggyGateView));
  });

  const textarea = tree.root.findByProps({ 'data-testid': 'feedback' });

  // Simulate the user typing "hello" one char at a time.
  for (const ch of 'hello') {
    act(() => {
      textarea.props.onChange({
        target: { value: textarea.props.value + ch },
      });
    });
  }

  const expected = 1; // only the initial render of HeavyArtifactViewer
  const observed = renderCount;

  const verdict = observed === expected ? 'PASS' : 'FAIL';
  console.log(
    `[repro] HeavyArtifactViewer renders after typing "hello":\n` +
    `         expected = ${expected}\n` +
    `         observed = ${observed}\n` +
    `         ${verdict}`,
  );

  if (observed !== expected) {
    console.error(
      '\n[repro] FAIL: typing in the feedback textarea re-rendered the\n' +
      '        heavy markdown viewer.\n' +
      '        Root cause: in src/components/GateView.tsx, the <textarea>\n' +
      '        and <ArtifactViewer> share a parent whose state updates on\n' +
      '        every keystroke, re-rendering the heavy ReactMarkdown subtree.\n' +
      '        Fix: wrap the heavy viewer in React.memo (or hoist the\n' +
      '        feedback state into its own child) so typing doesn\'t\n' +
      '        invalidate the markdown tree.',
    );
    process.exit(1);
  }
  process.exit(0);
}

main();
