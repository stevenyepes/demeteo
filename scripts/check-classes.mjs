/*
 * A gate against class-name rot: every class the UI uses must be a class the
 * stylesheet defines.
 *
 * Nothing else in this toolchain reads a class name. `tsc` sees a string,
 * Biome sees a string, Vitest renders the string into jsdom and asserts on the
 * DOM, never on whether a rule matched. Tailwind, for its part, emits a rule
 * for every candidate it *can* resolve and silently emits nothing for the rest
 * — a misspelled utility is not an error anywhere, it is an element with no
 * styling. So the failure is invisible in CI and nearly invisible on screen:
 * `font-outfit` on a heading inherits Inter from `body` and simply looks like a
 * heading someone chose not to style.
 *
 * That is not hypothetical. `font-outfit` (80 uses), `font-display` (18) and
 * `animate-fadeIn` (5) all rotted here and survived for months across 32 files.
 * The cause is structural rather than careless: Tailwind v4 derives
 * `font-<name>` from a `--font-<name>` key in `@theme`, so renaming or never
 * adding the key silently retires the utility, and the CSS side of the rename
 * leaves no trace on the call sites.
 *
 * So: compile the real stylesheet, collect the class selectors it emits,
 * collect the class names src/ puts in a class attribute, and fail on the
 * difference.
 *
 * ## Why it is narrow
 *
 * A naive version of this reports well over a hundred things, almost all junk,
 * and a gate that cries wolf is switched off inside a week. Two decisions keep
 * it quiet, both of them deliberate under-reporting:
 *
 * 1. Only strings in a **class attribute** are candidates. A class name held in
 *    a `const`, a lookup table, or returned by a helper is not seen. Widening
 *    this to every string literal in the tree means every event name, id and
 *    label becomes a candidate.
 *
 * 2. An unmatched token is reported only when its leading segment is a
 *    **namespace the stylesheet actually emits** — `font-outfit` is reported
 *    because `.font-heading` proves `font-` is a namespace here. This is
 *    check-doc-refs.sh's rule: report only where there is evidence the thing
 *    was meant to resolve. It is also what it costs — `prose prose-invert`
 *    (from a typography plugin this project does not install) is dead and is
 *    passed over, because no `prose-` rule exists to prove the namespace.
 *
 * Tokens carrying `[`, `(`, `%` or `#` are skipped too: arbitrary values are
 * spelled out by the author, Tailwind accepts almost anything inside them, and
 * they do not rot the way a `@theme` key does.
 *
 * ## The two things that make it hard, recorded so they are not undone
 *
 * **`font-display` is both a class and a real CSS property.** src/App.css sets
 * `font-display: block` inside the bundled Nerd Font `@font-face`. Any check
 * that greps the compiled sheet for the bare token finds that declaration and
 * concludes the utility exists. Only *selector* text is scanned here, so a
 * declaration can never be mistaken for a rule — and for the same reason, never
 * find-and-replace this name across the tree.
 *
 * **Escaped selectors, not `.name{`.** Tailwind writes `.text-\[10px\]`,
 * `.hover\:bg-white\/5:hover`, `:where(.space-y-2 > :not(:last-child))`. A
 * presence test that expects `.name` followed by `{` misses most real utilities
 * and reports them as dead. Selectors are therefore kept in escaped form and
 * candidates are escaped to match, which lines the two up exactly: a candidate
 * carries its own variants (`hover:bg-white/5`), and escaping turns them into
 * the literal prefix Tailwind wrote.
 *
 * `verifyExtractor()` pins both of these, plus the lexer, and runs on every
 * invocation — because the way this gate fails is by going quiet, and a quiet
 * pass would otherwise be indistinguishable from a correct one.
 *
 * `.design-sync/check-classes.mjs` is a separate, narrower sibling: it checks the
 * design-sync previews, which sit outside `@source` and so resolve against a
 * cached sheet rather than a compiled one. Different tree, different inputs.
 *
 * Usage:
 *   node scripts/check-classes.mjs                 # check src/
 *   node scripts/check-classes.mjs <dir> --debug    # …and list what was suppressed
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import { build } from "vite";

const ROOT = path.resolve(fileURLToPath(new URL("..", import.meta.url)));

const args = process.argv.slice(2);
const SRC = path.resolve(ROOT, args.find((a) => !a.startsWith("--")) ?? "src");
const DEBUG = args.includes("--debug");

const SOURCE_EXT = new Set([".tsx", ".ts", ".jsx", ".js", ".html"]);

// Classes Tailwind never emits a rule for, because their whole job is to be a
// selector other rules reach through (`group-hover:`, `peer-checked:`).
const MARKERS = /^(?:group|peer)(?:\/[A-Za-z0-9_-]+)?$/;

const ARBITRARY = /[[\]().,%#]/;

async function compileStylesheet(src) {
  const entry = path.join(src, "App.css");
  if (!existsSync(entry)) {
    throw new Error(`no stylesheet at ${entry}; this gate compiles <dir>/App.css`);
  }
  const out = await build({
    configFile: false,
    root: ROOT,
    logLevel: "silent",
    plugins: [tailwindcss()],
    build: {
      write: false,
      cssMinify: false,
      rollupOptions: { input: entry },
    },
  });
  const outputs = Array.isArray(out) ? out[0].output : out.output;
  const sheet = outputs.find((o) => o.fileName.endsWith(".css"));
  if (!sheet) throw new Error(`${entry} compiled to no stylesheet`);
  return String(sheet.source);
}

/** Selector text only: everything between a rule boundary and the `{` after it. */
function preludes(css) {
  const found = [];
  let buf = "";
  for (let i = 0; i < css.length; i++) {
    const c = css[i];
    if (c === "/" && css[i + 1] === "*") {
      const end = css.indexOf("*/", i + 2);
      if (end < 0) break;
      i = end + 1;
    } else if (c === '"' || c === "'") {
      const end = css.indexOf(c, i + 1);
      i = end < 0 ? css.length : end;
    } else if (c === "{") {
      found.push(buf);
      buf = "";
    } else if (c === "}" || c === ";") {
      buf = "";
    } else {
      buf += c;
    }
  }
  return found;
}

// Kept escaped. The token ends at the first *unescaped* `:` `(` `>` — which is
// what makes it comparable to an escaped candidate by equality:
// `.hover\:bg-white\/5:hover` yields `hover\:bg-white\/5`, exactly what
// `hover:bg-white/5` escapes to. Requiring the first character to be a letter,
// `-` or `_` is what keeps a length out: an at-rule prelude is scanned like any
// other, and `@supports (top:.5rem)` would otherwise define `5rem`.
const CLASS_SELECTOR = /\.((?:\\.|[A-Za-z_-])(?:\\.|[A-Za-z0-9_-])*)/g;

function definedClasses(css) {
  const names = new Set();
  for (const prelude of preludes(css)) {
    for (const m of prelude.matchAll(CLASS_SELECTOR)) names.add(m[1]);
  }
  return names;
}

function escapeClass(name) {
  return name.replace(/[^A-Za-z0-9_-]/g, (c) => `\\${c}`);
}

function unescapeClass(name) {
  return name.replace(/\\(.)/g, "$1");
}

function shared(a, b) {
  let i = 0;
  while (i < a.length && i < b.length && a[i] === b[i]) i++;
  return i;
}

/** The utility's leading segment, with variants and `-`/`!` modifiers removed. */
function utilityNamespace(name) {
  const base = name.slice(name.lastIndexOf(":") + 1).replace(/^[-!]+/, "");
  const dash = base.indexOf("-");
  return dash > 0 ? base.slice(0, dash) : null;
}

// A literal being *compared*, not applied: `className={side === "top-left" ? … : …}`.
// It sits inside the attribute like any other string, so the operator in front
// of it is the only thing that distinguishes it.
const COMPARISON = /(?:[=!]=|\.(?:includes|startsWith|endsWith|indexOf)\()$/;

const CLASS_ATTR = /(?<![A-Za-z0-9_$.])(?:className|class)\s*=\s*/g;

/**
 * String literals in class position, one attribute at a time.
 *
 * Anchoring on each `className=` instead of lexing the file front to back is
 * most of why this is quiet. A whole-file lexer has to know when it is inside
 * JSX *text*, where an apostrophe in ordinary prose ("don't") opens a string
 * that runs to the next apostrophe, swallowing braces and quotes until the
 * scanner is one quote out of phase — after which a closing `"` reads as an
 * opening one and the remainder of the render arrives as a single enormous
 * class literal full of markup. Parsing forward from a known-good anchor cannot
 * accumulate that: a mis-parse stops at the end of one attribute.
 *
 * Inside `{…}` the content is an expression, where apostrophes really are
 * strings and a small lexer is exact.
 */
function classLiterals(text) {
  const out = [];

  const readQuoted = (start) => {
    const quote = text[start];
    let body = "";
    let j = start + 1;
    for (; j < text.length; j++) {
      if (text[j] === "\\") j++;
      else if (text[j] === quote) break;
      else body += text[j];
    }
    return { body, next: j + 1 };
  };

  const keep = (body, at) => {
    if (!COMPARISON.test(text.slice(Math.max(0, at - 24), at).trimEnd())) {
      out.push({ body, at });
    }
  };

  for (const anchor of text.matchAll(CLASS_ATTR)) {
    let i = anchor.index + anchor[0].length;

    if (text[i] === '"' || text[i] === "'") {
      keep(readQuoted(i).body, i);
      continue;
    }
    if (text[i] !== "{") continue;

    // Balanced expression. A template literal becomes a frame of its own, so
    // its static text is collected while its interpolations are lexed as the
    // code they are, each contributing only the word boundary it stands for.
    const frames = [{ kind: "code", braces: 0 }];
    for (; i < text.length; i++) {
      const frame = frames[frames.length - 1];
      const c = text[i];

      if (frame.kind === "template") {
        if (c === "\\") i++;
        else if (c === "`") {
          frames.pop();
          keep(frame.body, frame.at);
        } else if (c === "$" && text[i + 1] === "{") {
          frame.body += " ";
          frames.push({ kind: "code", braces: 1 });
          i++;
        } else frame.body += c;
        continue;
      }

      if (c === "/" && text[i + 1] === "/") {
        const end = text.indexOf("\n", i);
        i = end < 0 ? text.length : end;
      } else if (c === "/" && text[i + 1] === "*") {
        const end = text.indexOf("*/", i + 2);
        i = end < 0 ? text.length : end + 1;
      } else if (c === '"' || c === "'") {
        const { body, next } = readQuoted(i);
        keep(body, i);
        i = next - 1;
      } else if (c === "`") {
        frames.push({ kind: "template", body: "", at: i });
      } else if (c === "{") {
        frame.braces++;
      } else if (c === "}" && --frame.braces === 0) {
        if (frames.length === 1) break;
        frames.pop();
      }
    }
  }
  return out;
}

function candidates(file) {
  const text = readFileSync(file, "utf8");
  const newlines = [];
  for (let i = 0; i < text.length; i++) if (text[i] === "\n") newlines.push(i);
  const lineAt = (offset) => {
    let lo = 0;
    let hi = newlines.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (newlines[mid] < offset) lo = mid + 1;
      else hi = mid;
    }
    return lo + 1;
  };

  const seen = new Set();
  const out = [];
  for (const lit of classLiterals(text)) {
    const line = lineAt(lit.at);
    for (const token of lit.body.split(/\s+/)) {
      const key = `${token}@${line}`;
      if (!token || seen.has(key)) continue;
      seen.add(key);
      out.push({ token, line });
    }
  }
  return out;
}

function sourceFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const p = path.join(dir, entry);
    if (statSync(p).isDirectory()) out.push(...sourceFiles(p));
    else if (SOURCE_EXT.has(path.extname(p))) out.push(p);
  }
  return out;
}

function verifyExtractor() {
  const fixture = [
    "const a = <p>Don't stop</p>;",
    'const b = <div className="font-heading text-md" />;',
    "const c = <div className={`p-2 ${on ? 'bg-cyan-500/10' : \"\"} font-outfit`} />;",
    'const d = <div className={side === "top-left" ? "mt-1" : "mb-1"} />;',
    'const e = <div className={cx("hover:bg-white/5", { "md:flex": wide })} />;',
  ].join("\n");
  const got = classLiterals(fixture)
    .flatMap((l) => l.body.split(/\s+/))
    .filter(Boolean)
    .sort()
    .join(" ");
  const want = [
    "bg-cyan-500/10",
    "font-heading",
    "font-outfit",
    "hover:bg-white/5",
    "mb-1",
    "md:flex",
    "mt-1",
    "p-2",
    "text-md",
  ]
    .sort()
    .join(" ");
  if (got !== want) {
    throw new Error(`class-attribute lexer drifted.\n  got:  ${got}\n  want: ${want}`);
  }

  // Every negative case here is a real shape from the compiled sheet that a
  // looser reading turns into a phantom utility: `font-display` the property,
  // `min-width` a bare token in an at-rule prelude, `.5rem` a length that opens
  // with a dot. `min-width` is the one that fires if the leading `.` ever stops
  // being required — which is the mistake that would also make the compiled
  // sheet appear to define `font-display`.
  const fixtureCss = [
    "@font-face{font-family:'X';font-display:block}",
    ".font-heading{font-family:var(--font-heading)}",
    ".text-\\[10px\\]{font-size:10px}",
    "@media (min-width:40rem){.md\\:flex{display:flex}}",
    "@supports (top:.5rem){.hover\\:bg-white\\/5:hover{background:#fff}}",
    ":where(.space-y-2 > :not(:last-child)){margin-top:.5rem}",
  ].join("\n");
  if (!fixtureCss.includes("font-display:")) {
    throw new Error("fixture no longer carries the font-display property; it proves nothing");
  }

  const fixtureDefs = definedClasses(fixtureCss);
  for (const [name, expected] of [
    ["font-heading", true],
    ["text-[10px]", true],
    ["md:flex", true],
    ["hover:bg-white/5", true],
    ["space-y-2", true],
    ["font-display", false],
    ["min-width", false],
    ["5rem", false],
  ]) {
    if (fixtureDefs.has(escapeClass(name)) !== expected) {
      throw new Error(
        `selector extraction drifted: \`${name}\` should be ${
          expected ? "defined" : "undefined"
        } in the fixture; got the opposite`,
      );
    }
  }
}

verifyExtractor();

const css = await compileStylesheet(SRC);
const defs = definedClasses(css);

const namespaces = new Set();
for (const name of defs) {
  const ns = utilityNamespace(unescapeClass(name));
  if (ns) namespaces.add(ns);
}

const reported = [];
const suppressed = [];
for (const file of sourceFiles(SRC)) {
  for (const { token, line } of candidates(file)) {
    if (MARKERS.test(token)) continue;
    if (defs.has(escapeClass(token))) continue;
    const ns = utilityNamespace(token);
    const rel = path.relative(ROOT, file);
    const row = { file: rel.startsWith("..") ? file : rel, line, token, ns };
    (ns && namespaces.has(ns) && !ARBITRARY.test(token) ? reported : suppressed).push(row);
  }
}

const byToken = new Map();
for (const row of reported) {
  if (!byToken.has(row.token)) byToken.set(row.token, []);
  byToken.get(row.token).push(row);
}
const ordered = [...byToken].sort((a, b) => b[1].length - a[1].length);

if (DEBUG) {
  const tally = new Map();
  for (const row of suppressed) tally.set(row.token, (tally.get(row.token) ?? 0) + 1);
  console.log(`${defs.size} selectors defined, in ${namespaces.size} namespaces`);
  console.log(`\n== reported (${reported.length}) ==`);
  for (const [token, rows] of ordered) console.log(`  ${rows.length}\t${token}`);
  console.log(`\n== suppressed (${suppressed.length}) ==`);
  for (const [token, n] of [...tally].sort((a, b) => b[1] - a[1])) console.log(`  ${n}\t${token}`);
  process.exit(0);
}

if (ordered.length === 0) {
  console.log(`class names resolve (${defs.size} selectors defined)`);
  process.exit(0);
}

for (const [token, rows] of ordered) {
  console.log(`  ${token}  (${rows.length} site${rows.length === 1 ? "" : "s"})`);
  for (const row of rows.slice(0, 4)) console.log(`      ${row.file}:${row.line}`);
  if (rows.length > 4) console.log(`      … and ${rows.length - 4} more`);
  const ns = rows[0].ns;
  const near = [...defs]
    .map(unescapeClass)
    .filter((n) => !n.includes(":") && !ARBITRARY.test(n) && utilityNamespace(n) === ns)
    .sort();
  // `text-` alone defines ninety utilities; printing them all is a wall nobody
  // reads. Nearest by shared prefix, which is what a typo or a retired
  // `@theme` key is closest to.
  const shown = [...near].sort((a, b) => shared(b, token) - shared(a, token)).slice(0, 8);
  if (near.length) {
    const more = near.length > shown.length ? `  … ${near.length} in total` : "";
    console.log(`      defined in \`${ns}-\`: ${shown.sort().join(" ")}${more}`);
  }
}

console.log();
console.log("  ^ used in a class attribute, but no rule defines them, so they style");
console.log("    nothing at all. Either switch to one of the names listed above, or");
console.log("    register the `@theme` key in src/App.css that Tailwind derives the");
console.log("    utility from — `--font-*` feeds `font-*`, `--animate-*` feeds");
console.log("    `animate-*`. Tailwind emits no rule and no warning for a candidate");
console.log("    it cannot resolve, which is why nothing else here reports this.");
console.log();
process.exit(1);
