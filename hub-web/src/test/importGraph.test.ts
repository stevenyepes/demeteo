/// <reference types="node" />

import { readFileSync, statSync } from 'node:fs';
import { resolve } from 'node:path';

import { describe, expect, it } from 'vitest';

import { findImportViolations, type ImportViolation, type SourceTree } from './importGraph';

const ROOT = 'src/app/root.ts';
const TAURI_BACKED = 'src/app/tauriBacked.ts';
const CORE = '@tauri-apps/api/core';
const INVOKE = `import { invoke } from '${CORE}';\n`;

/** Answers only for the paths it was given, so a wrong resolution reads as a miss. */
function treeOf(files: Record<string, string>): SourceTree {
  const byPath = new Map(Object.entries(files));
  return { read: (path) => byPath.get(path) ?? null };
}

/** `ROOT` holding `source`, beside a module that reaches Tauri if anything follows it. */
function walkRoot(source: string) {
  return findImportViolations([ROOT], treeOf({ [ROOT]: source, [TAURI_BACKED]: INVOKE }));
}

const viaTauriBacked = [{ kind: 'forbidden', specifier: CORE, chain: [ROOT, TAURI_BACKED] }];

describe('findImportViolations', () => {
  it('reports a Tauri import in the root itself', () => {
    expect(findImportViolations([ROOT], treeOf({ [ROOT]: INVOKE }))).toEqual({
      violations: [{ kind: 'forbidden', specifier: CORE, chain: [ROOT] }],
      visited: [ROOT],
    });
  });

  it('reports a transitive Tauri import with the whole importer chain', () => {
    const tree = treeOf({
      [ROOT]: "import { a } from './a';\n",
      'src/app/a.ts': "import { b } from './b';\nexport const a = b;\n",
      'src/app/b.ts': "import { open } from '@tauri-apps/plugin-dialog';\nexport const b = open;\n",
    });

    expect(findImportViolations([ROOT], tree)).toEqual({
      violations: [
        {
          kind: 'forbidden',
          specifier: '@tauri-apps/plugin-dialog',
          chain: [ROOT, 'src/app/a.ts', 'src/app/b.ts'],
        },
      ],
      visited: [ROOT, 'src/app/a.ts', 'src/app/b.ts'],
    });
  });

  it('reports a side-effect import', () => {
    expect(walkRoot(`import '${CORE}';\n`).violations).toEqual([
      { kind: 'forbidden', specifier: CORE, chain: [ROOT] },
    ]);
  });

  it.each([
    "export * from './tauriBacked';",
    "export * as backed from './tauriBacked';",
    "export { invoke } from './tauriBacked';",
  ])('follows the re-export %s', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    `const core = await import('${CORE}');`,
    `const core = await import(\n  "${CORE}",\n);`,
    `const core = await import(/* @vite-ignore */ '${CORE}');`,
    `const core = await import(\`${CORE}\`);`,
  ])('reports the literal dynamic import %j', (source) => {
    expect(walkRoot(source).violations).toEqual([
      { kind: 'forbidden', specifier: CORE, chain: [ROOT] },
    ]);
  });

  it.each([
    "import type { Backed } from './tauriBacked';",
    "import type Backed from './tauriBacked';",
    "import type * as backed from './tauriBacked';",
    "export type { Backed } from './tauriBacked';",
    "export type * from './tauriBacked';",
  ])('does not follow the type-only declaration %s', (source) => {
    expect(walkRoot(source)).toEqual({ violations: [], visited: [ROOT] });
  });

  it.each([
    "import { type A, type B } from './tauriBacked';",
    "import { type A as Alias, } from './tauriBacked';",
    "export { type A, type B } from './tauriBacked';",
  ])('does not follow %s, whose every specifier is inline type', (source) => {
    expect(walkRoot(source)).toEqual({ violations: [], visited: [ROOT] });
  });

  it.each([
    "import { type A, b } from './tauriBacked';",
    "import D, { type A } from './tauriBacked';",
    "import {} from './tauriBacked';",
    "import type from './tauriBacked';",
    "import { type as invoke } from './tauriBacked';",
  ])('follows %s, which keeps a value binding or the module itself', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it('counts import() in type position as a value edge', () => {
    expect(walkRoot("export type Backed = typeof import('./tauriBacked');\n").violations).toEqual(
      viaTauriBacked,
    );
  });

  it('parses an import list spread over several lines as one declaration', () => {
    const valueList = ['import {', '  alpha,', '  // why beta', '  beta,', "} from './tauriBacked';"];
    const typeList = ['import {', '  type Alpha,', '  type Beta,', "} from './tauriBacked';"];

    expect(walkRoot(valueList.join('\n')).violations).toEqual(viaTauriBacked);
    expect(walkRoot(valueList.join('\r\n')).violations).toEqual(viaTauriBacked);
    expect(walkRoot(typeList.join('\n'))).toEqual({ violations: [], visited: [ROOT] });
  });

  it('reports a relative specifier that resolves to nothing as unresolved', () => {
    expect(walkRoot("import { x } from './missing';\n")).toEqual({
      violations: [{ kind: 'unresolved', specifier: './missing', chain: [ROOT] }],
      visited: [ROOT],
    });
  });

  it('does not wrap a specifier that climbs above the repo root back into the tree', () => {
    const tree = treeOf({
      [ROOT]: "import { x } from '../../../outside';\n",
      'outside.ts': 'export const x = 1;\n',
    });

    expect(findImportViolations([ROOT], tree)).toEqual({
      violations: [{ kind: 'unresolved', specifier: '../../../outside', chain: [ROOT] }],
      visited: [ROOT],
    });
  });

  it('reports a root that is not in the tree instead of walking nothing', () => {
    expect(findImportViolations(['src/app/typo.ts'], treeOf({ [ROOT]: INVOKE }))).toEqual({
      violations: [{ kind: 'unresolved', specifier: 'src/app/typo.ts', chain: [] }],
      visited: [],
    });
  });

  it('resolves as the bundler does: extension, directory index, query suffix, parent directory', () => {
    const tree = treeOf({
      [ROOT]: [
        "import { X } from './x';",
        "import { dir } from './dir';",
        "import Worker from './w.js?worker';",
        "import { util } from '../shared/util';",
        "import { here } from '.';",
        "import { both } from './both';",
      ].join('\n'),
      'src/app/x.tsx': 'export const X = () => null;\n',
      'src/app/dir/index.ts': 'export const dir = 1;\n',
      'src/app/w.js': 'self.onmessage = () => {};\n',
      'src/shared/util.ts': 'export const util = 1;\n',
      'src/app/index.ts': 'export const here = 1;\n',
      'src/app/both.ts': 'export const both = 1;\n',
      'src/app/both.js': 'export const both = 1;\n',
    });

    expect(findImportViolations([ROOT], tree)).toEqual({
      violations: [],
      visited: [
        ROOT,
        'src/app/x.tsx',
        'src/app/dir/index.ts',
        'src/app/w.js',
        'src/shared/util.ts',
        'src/app/index.ts',
        'src/app/both.js',
      ],
    });
  });

  it('never follows a bare specifier, and forbids the Tauri scope by exact name only', () => {
    const lookalikes = [
      "import React from 'react';",
      "import { like } from 'tauri-like';",
      "import { near } from '@tauri-apps-community/near';",
    ].join('\n');

    expect(walkRoot(lookalikes)).toEqual({ violations: [], visited: [ROOT] });
    expect(walkRoot("import tauri from '@tauri-apps';\n").violations).toEqual([
      { kind: 'forbidden', specifier: '@tauri-apps', chain: [ROOT] },
    ]);
  });

  it('terminates on an import cycle, visiting each module once', () => {
    const tree = treeOf({
      'src/app/a.ts': "import { b } from './b';\nexport const a = 1;\n",
      'src/app/b.ts': "import { a } from './a';\nexport const b = a;\n",
    });

    expect(findImportViolations(['src/app/a.ts'], tree)).toEqual({
      violations: [],
      visited: ['src/app/a.ts', 'src/app/b.ts'],
    });
  });

  it('visits a non-script file as a leaf without parsing it', () => {
    const tree = treeOf({
      [ROOT]: "import './App.css';\n",
      'src/app/App.css': '@import "./missing.css";\n',
    });

    expect(findImportViolations([ROOT], tree)).toEqual({
      violations: [],
      visited: [ROOT, 'src/app/App.css'],
    });
  });

  it('reports a specifier once per importer', () => {
    const twice = `import { invoke } from '${CORE}';\nimport { listen } from '${CORE}';\n`;

    expect(walkRoot(twice).violations).toHaveLength(1);
  });

  it('ignores import-shaped text in a comment that starts its line', () => {
    const commented = [
      `// import { invoke } from '${CORE}';`,
      '/**',
      ` * import { invoke } from '${CORE}';`,
      ' */',
      '/*',
      `import '${CORE}';`,
      '*/',
      `  /* import('${CORE}') */`,
      'export const quiet = 1;',
    ].join('\n');

    expect(walkRoot(commented)).toEqual({ violations: [], visited: [ROOT] });
  });

  it.each([
    `/* legacy */ import '${CORE}';`,
    `export const late = 1; // import('${CORE}')`,
    `export const text = "import '${CORE}'";`,
  ])('still reads %s, where the text does not sit in a leading comment', (source) => {
    expect(walkRoot(source).violations).toEqual([
      { kind: 'forbidden', specifier: CORE, chain: [ROOT] },
    ]);
  });

  it.each([
    "import {\n  a, // see {x}\n  b,\n} from './tauriBacked';",
    "import {\n  a, /* {b} */\n} from './tauriBacked';",
    "export {\n  a, // {x}\n} from './tauriBacked';",
  ])('follows %j, with a brace in a comment inside the clause', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    "import { a } /* c */ from './tauriBacked';",
    "import /* c */ { a } from './tauriBacked';",
    "import { a } from /* c */ './tauriBacked';",
    "import D, /* c */ { a } from './tauriBacked';",
    "await import( // hint\n  './tauriBacked'\n);",
    "import { a } from // see `docs`\n  './tauriBacked';",
    "import { a } from /* a */ './tauriBacked'; const s = /* b */ `x`;",
  ])('follows %j, with a comment between the tokens', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    "const css = `\n/* open\n`;\nimport { invoke } from './tauriBacked';\nconst x = 1; /* close */",
    "const css = `\n// open`; import './tauriBacked';",
  ])('reads %j as code, where a template literal may have opened the leading comment', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    "new Worker(new URL('./tauriBacked.ts', import.meta.url), { type: 'module' });",
    "const b = require('./tauriBacked');",
    "import b = require('./tauriBacked');",
    "label('from', require('./tauriBacked'));",
    "import defer * as backed from './tauriBacked';",
  ])('follows %s', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    `export const text = "copied from './tauriBacked'";`,
    "export const late = 1; // moved from './tauriBacked'",
  ])('reads %s as an import', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it.each([
    "import type {\n  A, // {x}\n} from './tauriBacked';",
    "import type { A } /* c */ from './tauriBacked';",
    "import {\n  type A, // {x}\n  type B, /* {y} */\n} from './tauriBacked';",
  ])('does not follow the type-only declaration %j, comments and all', (source) => {
    expect(walkRoot(source)).toEqual({ violations: [], visited: [ROOT] });
  });

  it.each([
    "import {\n  a, // import type { A,\n  B } from './tauriBacked';",
    "import {\n  a, /* export type { A, */ B } from './tauriBacked';",
  ])('follows %j, whose nearest keyword sits in a comment', (source) => {
    expect(walkRoot(source).violations).toEqual(viaTauriBacked);
  });

  it('does not read prose that quotes a path in backticks after the word from', () => {
    const prose = 'export const quiet = 1; // split out from `./tauriBacked`';

    expect(walkRoot(prose)).toEqual({ violations: [], visited: [ROOT] });
  });

  it.each([
    "const name = './tauriBacked';\nawait import(name);\nrequire(name);",
    "import.meta.glob('./tauriBacked.ts', { eager: true });",
    "new URL('./tauriBacked.ts', document.baseURI);",
    "const note = (\n  <p>\n    /* open\n  </p>\n);\nimport './tauriBacked';\nconst end = <p>{/* close */}</p>;",
    `const css = \`\n/* \${await import('./tauriBacked')} */\n\`;`,
    "const a = 1; /* note\n// still the note */ import './tauriBacked';",
    "const text = 'a \\\n// b'; import './tauriBacked';",
  ])('does not see the edge in %j', (source) => {
    expect(walkRoot(source)).toEqual({ violations: [], visited: [ROOT] });
  });
});

const REAL_ROOTS = ['hub-web/src/main.tsx', 'src/components/canvas/WorkflowCanvas.tsx'];

/**
 * The checkout itself. The walker probes `./dir` as a file before it tries
 * `dir/index.*`, and reading a directory throws, so anything that is not a
 * regular file answers as absent.
 */
const checkout: SourceTree = {
  read(path) {
    const file = resolve(process.cwd(), ...path.split('/'));
    try {
      return statSync(file).isFile() ? readFileSync(file, 'utf8') : null;
    } catch {
      return null;
    }
  },
};

function describeViolation({ kind, specifier, chain }: ImportViolation): string {
  return `${kind}: ${[...chain, specifier].join(' -> ')}`;
}

describe('the real tree, walked from hub-web and the canvas', () => {
  it('reaches no @tauri-apps module and leaves no relative import unresolved', () => {
    const { violations } = findImportViolations(REAL_ROOTS, checkout);

    expect(violations.map(describeViolation)).toEqual([]);
  });

  it('walked the shell, the canvas and the node catalog, and stopped short of the IPC layer', () => {
    const { visited } = findImportViolations(REAL_ROOTS, checkout);

    expect(visited).toEqual(
      expect.arrayContaining([
        'hub-web/src/App.tsx',
        'src/components/canvas/nodeCatalog.ts',
        'src/components/canvas/nodes/WorkflowNode.tsx',
      ]),
    );
    expect(visited).not.toContain('src/lib/workflows.ts');
  });
});
