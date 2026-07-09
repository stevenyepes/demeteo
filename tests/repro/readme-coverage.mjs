/**
 * README coverage report — keeps the README honest about the repo.
 *
 * Bug surface:
 *   The README claims a fixed set of concrete things — doc links, npm
 *   scripts, cargo commands, the SQLite DB path, four agent CLIs, and
 *   five architecture-diagram identifiers. Each can silently drift away
 *   from the repo: a renamed file, a deleted npm script, an agent
 *   adapter switched to a different CLI flag, a Tauri identifier
 *   change. By the time a contributor notices, the README has been
 *   misleading users for weeks. There is no existing guard.
 *
 * What this check asserts:
 *   For every concrete claim the README makes across six categories,
 *   the referenced artifact (file / script / command / identifier) is
 *   still present in the working tree. The script prints a fixed-width
 *   table with one row per claim, a final `Coverage: X / Y (NN.NN%)`
 *   line, and exits 1 when any claim is missing or stale.
 *
 * Categories and rules:
 *   1. Doc links      — every `](foo.md)` link target exists on disk.
 *   2. npm scripts    — every `npm run <name>` references a script
 *                       defined in `package.json` `scripts`.
 *   3. npx tools      — every `npx <tool>` refers to a devDependency
 *                       in `package.json` (or a Node-built-in).
 *   4. Cargo commands — every `cargo <sub>` is a recognised cargo
 *                       subcommand (whitelisted set: build, check, fmt,
 *                       clippy, test, bench, doc, install, run, …).
 *   5. Database path  — the `~/.local/share/<id>/demeteo.db` claim's
 *                       `<id>` matches the `identifier` in
 *                       `src-tauri/tauri.conf.json`.
 *   6. Agent CLIs     — each row's CLI invocation appears verbatim in
 *                       the Rust source under
 *                       `crates/demeteo-core/src/adapters/agent/`.
 *   7. Architecture   — each diagram identifier is exported as a
 *                       `pub struct` / `pub trait` / `pub fn` /
 *                       `pub mod` from a tracked `.rs` file in the
 *                       workspace.
 *
 * Run (no project deps — pure Node ≥ 18):
 *
 *   $ node tests/repro/readme-coverage.mjs
 *
 * Exit codes:
 *   0 — every claim verified.
 *   1 — at least one MISSING / STALE row.
 *
 *   The script is read-only over the repo: it never writes to
 *   `README.md` or any tracked file. Updating the README is a
 *   separate concern; this check only flags drift.
 */

import { readFileSync, existsSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

// --------------------------------------------------------------------------
// Repo-root resolution
// --------------------------------------------------------------------------
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = dirname(dirname(HERE)); // tests/repro -> repo root

// --------------------------------------------------------------------------
// Pure parsers (no I/O side effects)
// --------------------------------------------------------------------------

/**
 * Parse every Markdown link whose target ends in `.md` (with optional
 * `#anchor` and/or query string).
 *
 * Each entry is `{ target, line }` where `target` is the literal string
 * inside the parentheses (including anchor), and `line` is the 1-based
 * README line number — useful for surfacing where the claim lives.
 *
 * External `http(s)://...` links are skipped — they're not claims about
 * the repo.
 */
function parseDocLinks(readme) {
  const lines = readme.split('\n');
  const links = [];
  const re = /\[[^\]]*\]\(([^)]+)\)/g;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    let match;
    re.lastIndex = 0;
    while ((match = re.exec(line)) !== null) {
      const target = match[1];
      if (/^https?:\/\//i.test(target)) continue;
      if (!target.endsWith('.md')) continue;
      links.push({ target, line: i + 1 });
    }
  }
  return links;
}

/**
 * Resolve a Markdown link target like `docs/ARCHITECTURE.md#§1.2` to the
 * on-disk path (anchor stripped). Returns the path as it would appear
 * if present, relative to the repo root.
 */
function linkTargetToPath(target) {
  return target.split('#')[0].split('?')[0];
}

/**
 * Parse every `` `npm run <name>` `` and `` `npx <tool>` `` invocation
 * from the README's *Development* table.
 *
 * Each entry is `{ kind, name, raw, line }` where:
 *   - `kind` is `"npm-run"` or `"npx"`.
 *   - `name` is the script/tool identifier immediately after `npm run` /
 *     `npx` (everything up to the next whitespace).
 *   - `raw` is the full backtick-stripped command string.
 */
/**
 * Well-known `npx <tool>` → npm package mappings. Some packages expose
 * a binary under a different name than the package itself (the most
 * common case is `typescript`, whose `bin.tsc` provides the `tsc`
 * command). The README's *Development* table uses `npx tsc --noEmit`;
 * we resolve it to `typescript` here so the check stays correct.
 */
const NPX_TOOL_PACKAGE = {
  tsc: 'typescript',
  tsserver: 'typescript',
};

function parseShellCommands(readme) {
  const lines = readme.split('\n');
  const out = [];
  const re = /`(npm\s+run\s+([a-zA-Z0-9:_-]+)|npx\s+([a-zA-Z0-9:_-]+))([^`]*)`/g;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    let match;
    re.lastIndex = 0;
    while ((match = re.exec(line)) !== null) {
      const [, full, npmName, npxName, rest] = match;
      if (npmName) {
        out.push({
          kind: 'npm-run',
          name: npmName,
          raw: `npm run ${npmName}${rest}`,
          line: i + 1,
        });
      } else if (npxName) {
        out.push({
          kind: 'npx',
          name: npxName,
          raw: `npx ${npxName}${rest}`,
          line: i + 1,
        });
      }
      // full is unused outside this destructuring; keep the slot for clarity.
      void full;
    }
  }
  return out;
}

/**
 * Parse every `` `cargo <subcommand> ...` `` from the README. Each entry
 * is `{ sub, raw, line }`.
 */
function parseCargoCommands(readme) {
  const lines = readme.split('\n');
  const out = [];
  const re = /`cargo\s+([a-zA-Z][a-zA-Z0-9_-]*)([^`]*)`/g;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    let match;
    re.lastIndex = 0;
    while ((match = re.exec(line)) !== null) {
      out.push({
        sub: match[1],
        raw: `cargo ${match[1]}${match[2]}`,
        line: i + 1,
      });
    }
  }
  return out;
}

/**
 * Parse the literal SQLite DB path claim from the README. The README
 * anchors the path on the stable `~/.local/share/<id>/demeteo.db`
 * convention (Tauri's `app_local_data_dir()` on Linux).
 */
function parseDbPathClaim(readme) {
  const match = readme.match(/`~\/\.local\/share\/([^/]+)\/demeteo\.db`/);
  if (!match) return null;
  return { identifier: match[1], raw: match[0].replace(/`/g, '') };
}

/**
 * Parse the *Supported agents* table. Each row contributes
 * `{ agent, cli, line }` where `cli` is the literal backtick-stripped
 * command string and `line` is its README line number.
 */
function parseAgentClis(readme) {
  const lines = readme.split('\n');
  const out = [];
  const startIdx = lines.findIndex((l) => /^\s*\|.*Agent.*CLI invocation/i.test(l));
  if (startIdx < 0) return out;
  // Skip the header row + the `|-------|----|` separator row.
  for (let i = startIdx + 2; i < lines.length; i += 1) {
    const line = lines[i];
    const trimmed = line.trim();
    if (!trimmed.startsWith('|')) break;
    const cells = trimmed.split('|').slice(1, -1).map((c) => c.trim());
    if (cells.length < 2) continue;
    const [agentCell, cliCell] = cells;
    const agentMatch = agentCell.match(/\[?([a-zA-Z0-9_-]+)\]?/);
    const cliMatch = cliCell.match(/`([^`]+)`/);
    if (!agentMatch || !cliMatch) continue;
    out.push({
      agent: agentMatch[1],
      cli: cliMatch[1],
      line: i + 1,
    });
  }
  return out;
}

/**
 * Architecture-diagram terms the README names in the boxed diagram.
 * These are the canonical names the README pins; the check verifies
 * each is exported from a tracked `.rs` file in the workspace.
 */
const ARCHITECTURE_TERMS = [
  'StepExecutor',
  'AgentRuntime',
  'UnifiedCliRuntime',
  'WorktreeOpsPort',
  'MrPublisher',
];

/**
 * Standard cargo subcommands we accept. The README's *Development*
 * table only references a subset; the whitelist lets us flag a typo
 * (e.g. `cargo format`) the same way we flag a missing file. Source:
 * `cargo --list` core commands.
 */
const CARGO_SUBCOMMAND_WHITELIST = new Set([
  'bench',
  'build',
  'check',
  'clean',
  'clippy',
  'doc',
  'fetch',
  'fmt',
  'init',
  'install',
  'login',
  'logout',
  'metadata',
  'new',
  'owner',
  'package',
  'publish',
  'run',
  'rustc',
  'rustdoc',
  'search',
  'test',
  'uninstall',
  'update',
  'vendor',
  'verify-project',
  'version',
  'yank',
]);

// --------------------------------------------------------------------------
// Verifiers (read-only I/O over the repo)
// --------------------------------------------------------------------------

/**
 * Walk `dir` recursively, yielding absolute paths to regular files that
 * match `predicate(path)` returning true. Skips `.git`, `node_modules`,
 * `target`, and `vendor` — the same exclusion list the rest of the
 * project uses for source-only operations.
 */
function walkRustFiles(dir, predicate) {
  const out = [];
  const SKIP = new Set(['.git', 'node_modules', 'target', 'vendor', '.next']);
  const stack = [dir];
  while (stack.length > 0) {
    const current = stack.pop();
    let entries;
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (SKIP.has(entry.name)) continue;
      const abs = join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(abs);
      } else if (entry.isFile()) {
        if (predicate(abs)) out.push(abs);
      }
    }
  }
  return out;
}

/**
 * Read a JSON file from disk. Throws on missing / malformed JSON.
 */
function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/**
 * Verify a single doc-link claim: the target path exists relative to
 * the repo root.
 */
function verifyDocLink(repoRoot, link) {
  const path = linkTargetToPath(link.target);
  const abs = join(repoRoot, path);
  if (!existsSync(abs)) {
    return { status: 'MISSING', detail: `no such file: ${path}` };
  }
  return { status: 'PASS', detail: `exists: ${path}` };
}

/**
 * Verify a single `npm run <name>` claim: the name appears in
 * `package.json` `scripts`. Also accepts the first positional arg
 * pattern (e.g. `npm run tauri build`) by matching on the leading
 * token.
 */
function verifyNpmScript(repoRoot, claim, pkgScripts) {
  if (Object.prototype.hasOwnProperty.call(pkgScripts, claim.name)) {
    return {
      status: 'PASS',
      detail: `package.json scripts.${claim.name}`,
    };
  }
  return {
    status: 'MISSING',
    detail: `package.json has no scripts.${claim.name}`,
  };
}

/**
 * Verify a single `npx <tool>` claim: the tool is in devDependencies
 * or dependencies (i.e. reachable after `npm install`). Built-in tools
 * like `tsc` are reached through `typescript` (a devDependency) so this
 * is the right check.
 */
function verifyNpxTool(repoRoot, claim, pkg) {
  const all = { ...(pkg.dependencies || {}), ...(pkg.devDependencies || {}) };
  if (Object.prototype.hasOwnProperty.call(all, claim.name)) {
    return { status: 'PASS', detail: `package.json lists ${claim.name}` };
  }
  // Fall back to the well-known tool→package mapping (e.g. `tsc` is
  // shipped by the `typescript` package, whose `bin` field exposes
  // `tsc`).
  const mapped = NPX_TOOL_PACKAGE[claim.name];
  if (mapped && Object.prototype.hasOwnProperty.call(all, mapped)) {
    return {
      status: 'PASS',
      detail: `package.json lists ${mapped} (provides '${claim.name}')`,
    };
  }
  return {
    status: 'MISSING',
    detail: `package.json has no dependency ${claim.name}`,
  };
}

/**
 * Verify a single `cargo <sub>` claim: the subcommand is in the
 * standard whitelist. We don't shell out — `cargo --list` would add a
 * runtime dependency on the toolchain.
 */
function verifyCargoSubcommand(repoRoot, claim) {
  if (CARGO_SUBCOMMAND_WHITELIST.has(claim.sub)) {
    return { status: 'PASS', detail: `cargo subcommand: ${claim.sub}` };
  }
  return {
    status: 'MISSING',
    detail: `unrecognised cargo subcommand: ${claim.sub}`,
  };
}

/**
 * Verify the SQLite DB path claim: the `com.stvcloud.demeteo`
 * identifier in the README matches `identifier` in
 * `src-tauri/tauri.conf.json`. A mismatch is reported as STALE — the
 * README still names the old identifier.
 */
function verifyDbPath(repoRoot, claim) {
  const confPath = join(repoRoot, 'src-tauri', 'tauri.conf.json');
  if (!existsSync(confPath)) {
    return {
      status: 'MISSING',
      detail: `no tauri.conf.json at src-tauri/tauri.conf.json`,
    };
  }
  const conf = readJson(confPath);
  const actual = conf?.identifier;
  if (typeof actual !== 'string') {
    return {
      status: 'MISSING',
      detail: `tauri.conf.json has no identifier`,
    };
  }
  if (actual === claim.identifier) {
    return {
      status: 'PASS',
      detail: `identifier matches tauri.conf.json (${actual})`,
    };
  }
  return {
    status: 'STALE',
    detail: `README says '${claim.identifier}' but tauri.conf.json says '${actual}'`,
  };
}

/**
 * Verify an agent CLI claim: the literal CLI string from the README's
 * *Supported agents* table appears in some `.rs` file under
 * `crates/demeteo-core/src/adapters/agent/`. We don't require an exact
 * match — adapter args change over time — but the canonical form must
 * survive as a comment or string literal so the README stays
 * self-consistent.
 */
function verifyAgentCli(repoRoot, claim, agentFiles) {
  const tokens = claim.cli.split(/\s+/).filter(Boolean);
  if (tokens.length === 0) {
    return {
      status: 'MISSING',
      detail: `empty CLI claim for ${claim.agent}`,
    };
  }
  for (const file of agentFiles) {
    const text = readFileSync(file, 'utf8');
    let allFound = true;
    for (const token of tokens) {
      if (!tokenRegex(token).test(text)) {
        allFound = false;
        break;
      }
    }
    if (allFound) {
      return {
        status: 'PASS',
        detail: relative(repoRoot, file),
      };
    }
  }
  const missing = [];
  for (const file of agentFiles) {
    const text = readFileSync(file, 'utf8');
    for (const token of tokens) {
      if (!tokenRegex(token).test(text)) missing.push(token);
    }
    if (missing.length > 0) break;
  }
  return {
    status: 'MISSING',
    detail: `no agent source contains all tokens of '${claim.cli}' (missing: ${missing.join(', ')})`,
  };
}

/**
 * Verify an architecture-diagram term: the identifier is exported as a
 * `pub` item from a tracked `.rs` file in the workspace. We allow
 * `pub struct` / `pub trait` / `pub fn` / `pub mod` / `pub enum` /
 * `pub type` — whichever Rust's `pub` surface exposes.
 */
function verifyArchitectureTerm(repoRoot, term, rustFiles) {
  const re = new RegExp(
    `\\bpub\\s+(?:struct|trait|fn|mod|enum|type)\\s+${escapeRegExp(term)}\\b`,
  );
  for (const file of rustFiles) {
    const text = readFileSync(file, 'utf8');
    if (re.test(text)) {
      return {
        status: 'PASS',
        detail: relative(repoRoot, file),
      };
    }
  }
  return {
    status: 'MISSING',
    detail: `no .rs file exports 'pub ... ${term}'`,
  };
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Build a "word-boundary" regex for `token` that treats `-` as part of
 * the identifier. The default `\b` boundary only triggers between
 * `\w` (`[A-Za-z0-9_]`) and `\W` chars — so `\b--print\b` never
 * matches, because `-` is non-word on both sides. CLI flags like
 * `--print` and `--output-format` need a hyphen-aware boundary.
 */
function tokenRegex(token) {
  return new RegExp(
    `(?:^|[^A-Za-z0-9_-])${escapeRegExp(token)}(?=$|[^A-Za-z0-9_-])`,
  );
}

/**
 * Pad a string to `width` with trailing spaces. Mirrors the column
 * sizing the existing `tests/repro/*.mjs` scripts print.
 */
function pad(value, width) {
  const s = String(value);
  return s.length >= width ? s : s + ' '.repeat(width - s.length);
}

/**
 * Render the coverage report table. Returns the formatted string. The
 * caller prints it.
 */
function renderReport(rows) {
  const widths = {
    category: Math.max(8, ...rows.map((r) => r.category.length)),
    reference: Math.max(9, ...rows.map((r) => r.reference.length)),
    status: Math.max(6, ...rows.map((r) => r.status.length)),
    detail: 0, // detail takes the remaining width
  };

  const headerCells = [
    pad('Category', widths.category),
    pad('Reference', widths.reference),
    pad('Status', widths.status),
    'Detail',
  ];
  const sep = [
    '-'.repeat(widths.category),
    '-'.repeat(widths.reference),
    '-'.repeat(widths.status),
    '-'.repeat(40),
  ];
  const lines = [headerCells.join(' | '), sep.join('-+-')];
  for (const row of rows) {
    lines.push(
      [
        pad(row.category, widths.category),
        pad(row.reference, widths.reference),
        pad(row.status, widths.status),
        row.detail,
      ].join(' | '),
    );
  }
  return lines.join('\n');
}

// --------------------------------------------------------------------------
// Driver
// --------------------------------------------------------------------------

function main() {
  // Optional CLI override: `node tests/repro/readme-coverage.mjs [path]`
  // — useful for drift-testing the script with a deliberately-broken
  // copy of the README without touching the real one. The CI workflow
  // invokes the script with no args.
  const cliArg = process.argv[2];
  const readmePath = cliArg
    ? join(REPO_ROOT, cliArg)
    : join(REPO_ROOT, 'README.md');
  if (!existsSync(readmePath)) {
    console.error(`[readme-coverage] No README at ${readmePath}`);
    process.exit(2);
  }
  const readme = readFileSync(readmePath, 'utf8');
  const pkg = readJson(join(REPO_ROOT, 'package.json'));
  const pkgScripts = pkg.scripts || {};

  const rustRoots = [
    join(REPO_ROOT, 'src-tauri', 'src'),
    join(REPO_ROOT, 'crates'),
  ];
  const rustFiles = [];
  for (const root of rustRoots) {
    if (!existsSync(root)) continue;
    if (statSync(root).isDirectory()) {
      rustFiles.push(
        ...walkRustFiles(root, (p) => p.endsWith('.rs')),
      );
    }
  }
  const agentFiles = rustFiles.filter((p) =>
    p.includes(`${REPO_ROOT}/crates/demeteo-core/src/adapters/agent/`.replace(`${REPO_ROOT}/`, '')) ||
    p.includes('/crates/demeteo-core/src/adapters/agent/'),
  );

  const rows = [];

  // 1. Doc links
  for (const link of parseDocLinks(readme)) {
    const result = verifyDocLink(REPO_ROOT, link);
    rows.push({
      category: 'Doc links',
      reference: `${link.target}  (L${link.line})`,
      ...result,
    });
  }

  // 2 & 3. npm scripts and npx tools
  for (const claim of parseShellCommands(readme)) {
    if (claim.kind === 'npm-run') {
      const result = verifyNpmScript(REPO_ROOT, claim, pkgScripts);
      rows.push({
        category: 'npm scripts',
        reference: `${claim.raw}  (L${claim.line})`,
        ...result,
      });
    } else {
      const result = verifyNpxTool(REPO_ROOT, claim, pkg);
      rows.push({
        category: 'npx tools',
        reference: `${claim.raw}  (L${claim.line})`,
        ...result,
      });
    }
  }

  // 4. Cargo commands
  for (const claim of parseCargoCommands(readme)) {
    const result = verifyCargoSubcommand(REPO_ROOT, claim);
    rows.push({
      category: 'Cargo cmds',
      reference: `${claim.raw}  (L${claim.line})`,
      ...result,
    });
  }

  // 5. Database path
  const dbClaim = parseDbPathClaim(readme);
  if (dbClaim) {
    const result = verifyDbPath(REPO_ROOT, dbClaim);
    rows.push({
      category: 'DB path',
      reference: dbClaim.raw,
      ...result,
    });
  }

  // 6. Agent CLIs
  for (const claim of parseAgentClis(readme)) {
    const result = verifyAgentCli(REPO_ROOT, claim, agentFiles);
    rows.push({
      category: 'Agent CLIs',
      reference: `${claim.agent}: ${claim.cli}  (L${claim.line})`,
      ...result,
    });
  }

  // 7. Architecture diagram terms
  for (const term of ARCHITECTURE_TERMS) {
    const result = verifyArchitectureTerm(REPO_ROOT, term, rustFiles);
    rows.push({
      category: 'Architecture',
      reference: term,
      ...result,
    });
  }

  // Compute coverage.
  const total = rows.length;
  const passed = rows.filter((r) => r.status === 'PASS').length;
  const pct = total === 0 ? 100 : (passed / total) * 100;

  console.log(renderReport(rows));
  console.log('');
  console.log(`Coverage: ${passed} / ${total} (${pct.toFixed(2)}%)`);

  const failed = rows.filter((r) => r.status !== 'PASS').length;
  if (failed > 0) {
    console.error(
      `\n[readme-coverage] FAIL: ${failed} claim(s) did not verify — README is out of sync with the repo.`,
    );
    process.exit(1);
  }
  process.exit(0);
}

main();