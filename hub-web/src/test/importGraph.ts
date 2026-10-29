/**
 * The gate that keeps `@tauri-apps` out of everything hub-web can reach: a
 * walk of the value-import graph, read from source text.
 *
 * Static, because a test that imports and renders cannot see what it would be
 * looking for. hub-web's tests run under `src/test/setup.ts`, which
 * `vi.mock`s `@tauri-apps/api/core` for the whole suite, so a module that
 * reaches Tauri loads and renders cleanly there and fails only in a browser.
 *
 * Lexical, because TypeScript 7 may not ship the compiler API a real parse
 * needs and the gate adds no dependency to get one. A false report costs one
 * word in the importer and a missed edge costs the gate, so an edge is read
 * from its specifier alone and nothing around it has to parse:
 *
 * - `from '<literal>'`, `import '<literal>'`, `import('<literal>')`,
 *   `require('<literal>')` and `new URL('<literal>', import.meta.url)` are
 *   edges, with whitespace and comments of either form between the tokens.
 * - The binding clause is read only to exempt. A `from` edge is skipped when
 *   the text back to the nearest `import` / `export` parses, comments removed,
 *   as a type-only clause, and that keyword cannot itself sit in a comment. A
 *   clause that does not parse is followed.
 * - `import { T } from './x'` is a value edge even when `T` is only ever used
 *   as a type and the bundler would erase it. The fix is the word `type`,
 *   after which the edge's safety stops resting on usage analysis.
 * - `import('./x')` is a value edge in type position too.
 * - A comment is recognised only where it starts its line, and not even there
 *   when it holds a backtick: a template literal may have opened it, and the
 *   backtick that closes the template is then inside. Import-shaped text in
 *   any other comment or in a string is read as an import, `from './x'` in a
 *   sentence included. Doing better means tracking strings, which means
 *   telling a regex from a division and JSX text from code; a scanner that
 *   loses its place there skips real imports, and this one keeps no place to
 *   lose.
 *
 * What it does not see:
 *
 * - A specifier that is not a literal: `import(name)`, `require(name)`,
 *   `import.meta.glob`, and a `new URL` whose base is not `import.meta.url`
 *   written in place.
 * - Anything a path alias would resolve: only relative specifiers are
 *   followed, and the repo has no alias today.
 * - Code inside a leading comment that no comment opened. A `/*` or `//` that
 *   starts a line of JSX text, of a template literal ahead of a `${}`, of a
 *   block comment opened mid-line, or of a string continued with a backslash
 *   hides everything up to where a comment starting there would end, unless a
 *   backtick falls inside.
 *
 * Pure on purpose: no `node:fs`, no `process`. Paths are repo-relative and
 * `/`-separated, so a fixture describes the same graph on every OS, and the
 * caller supplies the tree.
 */

export interface ImportViolation {
  kind: 'forbidden' | 'unresolved';
  specifier: string;
  /** Root first, importer last; repo-relative, `/`-separated on every OS. */
  chain: string[];
}

export interface SourceTree {
  /** null when no file exists at that repo-relative path. */
  read(path: string): string | null;
}

interface Module {
  path: string;
  source: string;
  chain: string[];
}

const FORBIDDEN_SCOPE = '@tauri-apps';

/**
 * Vite's default `resolve.extensions`, in Vite's order: where two files share
 * a stem, the one to read is the one the bundle would contain.
 */
const EXTENSIONS = ['.mjs', '.js', '.mts', '.ts', '.jsx', '.tsx', '.json'];

const SCRIPT = /\.[cm]?[jt]sx?$/;

const LEADING_COMMENT = /^[ \t]*(?:\/\/.*|\/\*[\s\S]*?\*\/)/gm;

/**
 * Neither form backtracks: a line comment runs to the end of its line and a
 * block comment to its first `*\/`. `EDGE` rejects a backtick after `from`,
 * and a comment free to shrink or stretch finds one to stop in front of —
 * inside itself for a line comment that quotes a name, past a later comment
 * for a block — which drops the real import in between.
 */
const COMMENT = String.raw`\/\/.*$|\/\*(?:[^*]|\*(?!\/))*\*\/`;
const GAP = String.raw`(?:\s|${COMMENT})*`;

const WORD = String.raw`[\w$]+`;
const LITERAL = String.raw`['"\x60][^'"\x60\n]*['"\x60]`;
const QUOTED = String.raw`['"\x60]([^'"\x60\n]*)['"\x60]`;
const BINDINGS = String.raw`(?:type\s+)?(?:${WORD}\s*,\s*)?(?:\{[^{}]*\}|\*(?:\s*as\s+${WORD})?)|(?:type\s+)?${WORD}`;

/**
 * Group 1 is set for a `from` edge, the only kind a binding clause can exempt;
 * group 2 is the specifier.
 *
 * The specifier sits in a lookahead so that a match consumes its head and
 * nothing else. Consumed, the quote that closes `'from'` in
 * `label('from', require('./x'))` pairs with the one that opens `'./x'` and
 * the real head between them is never scanned.
 *
 * A backtick is a literal only in a call. After `from` or a bare `import` it
 * is prose, and comments here quote paths that way.
 */
const EDGE = new RegExp(
  String.raw`(?<![\w$])(?:(?:(from)|import)(?!${GAP}\x60)|(?:import|require)${GAP}\(|new${GAP}URL${GAP}\((?=${GAP}${LITERAL}${GAP},${GAP}import\.meta\.url))(?=${GAP}${QUOTED})`,
  'gm',
);

const CLAUSE = new RegExp(String.raw`^(?:${BINDINGS})$`);
const COMMENTS = new RegExp(COMMENT, 'gm');

/** Group 1 is the text before the last `import` / `export`, group 2 the text after it. */
const LAST_DECLARATION = /^([\s\S]*)(?<![\w$])(?:import|export)(?![\w$])([\s\S]*)$/;

/** `{ type as x }` binds a value named `type`, so `type as` never marks a type. */
const INLINE_TYPE = /^type\s+(?!as\b)[\w$]/;

export function findImportViolations(
  roots: string[],
  tree: SourceTree,
): { violations: ImportViolation[]; visited: string[] } {
  const violations: ImportViolation[] = [];
  const queued = new Set<string>();
  const queue: Module[] = [];

  for (const root of roots) {
    const path = normalize(root);
    const source = path === null ? null : tree.read(path);
    if (path === null || source === null) {
      violations.push({ kind: 'unresolved', specifier: root, chain: [] });
    } else if (!queued.has(path)) {
      queued.add(path);
      queue.push({ path, source, chain: [path] });
    }
  }

  for (let next = 0; next < queue.length; next += 1) {
    const { path, source, chain } = queue[next];
    if (!SCRIPT.test(path)) continue;

    for (const specifier of valueSpecifiers(source)) {
      if (isForbidden(specifier)) {
        violations.push({ kind: 'forbidden', specifier, chain });
      } else if (isRelative(specifier)) {
        const target = resolve(path, specifier, tree);
        if (target === null) {
          violations.push({ kind: 'unresolved', specifier, chain });
        } else if (!queued.has(target.path)) {
          queued.add(target.path);
          queue.push({ ...target, chain: [...chain, target.path] });
        }
      }
    }
  }

  return { violations, visited: queue.map((module) => module.path) };
}

function valueSpecifiers(source: string): string[] {
  const code = source.replace(LEADING_COMMENT, (comment) =>
    comment.includes('`') ? comment : '',
  );
  const specifiers = new Set<string>();
  for (const match of code.matchAll(EDGE)) {
    const isFrom = match[1] !== undefined;
    if (!isFrom || !endsInTypeOnlyClause(code.slice(0, match.index))) specifiers.add(match[2]);
  }
  return [...specifiers];
}

/** `head` is the source up to a `from`. */
function endsInTypeOnlyClause(head: string): boolean {
  const declaration = LAST_DECLARATION.exec(head);
  if (declaration === null) return false;

  const [, before, rest] = declaration;
  if (endsInsideComment(before)) return false;
  const clause = rest.replace(COMMENTS, ' ').trim();
  return CLAUSE.test(clause) && isTypeOnly(clause);
}

/**
 * Whether a comment may still be open where `text` ends. A keyword found
 * there was not necessarily written as one: `// import type { A,` inside a
 * value import's braces would otherwise make the rest of them read type-only.
 */
function endsInsideComment(text: string): boolean {
  const line = text.slice(text.lastIndexOf('\n') + 1);
  const opened = text.lastIndexOf('/*');
  return line.includes('//') || (opened !== -1 && text.lastIndexOf('*/') < opened + 2);
}

function isTypeOnly(clause: string): boolean {
  if (/^type\s+[\w$*{]/.test(clause)) return true;

  const named = /^\{([^{}]*)\}$/.exec(clause);
  if (named === null) return false;
  const bindings = named[1]
    .split(',')
    .map((binding) => binding.trim())
    .filter((binding) => binding !== '');
  return bindings.length > 0 && bindings.every((binding) => INLINE_TYPE.test(binding));
}

function isForbidden(specifier: string): boolean {
  return specifier === FORBIDDEN_SCOPE || specifier.startsWith(`${FORBIDDEN_SCOPE}/`);
}

function isRelative(specifier: string): boolean {
  return /^\.\.?(?:\/|$)/.test(specifier);
}

function resolve(
  importer: string,
  specifier: string,
  tree: SourceTree,
): { path: string; source: string } | null {
  const directory = importer.slice(0, importer.lastIndexOf('/') + 1);
  const base = normalize(directory + specifier.split('?')[0]);
  if (base === null) return null;

  const candidates = [
    base,
    ...EXTENSIONS.map((extension) => base + extension),
    ...EXTENSIONS.map((extension) => `${base}/index${extension}`),
  ];
  for (const path of candidates) {
    const source = tree.read(path);
    if (source !== null) return { path, source };
  }
  return null;
}

/** null when the path climbs above the repo root. */
function normalize(path: string): string | null {
  const segments: string[] = [];
  for (const segment of path.split('/')) {
    if (segment === '..') {
      if (segments.length === 0) return null;
      segments.pop();
    } else if (segment !== '' && segment !== '.') {
      segments.push(segment);
    }
  }
  return segments.join('/');
}
