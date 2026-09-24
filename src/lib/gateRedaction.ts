/**
 * The floor under the gate fence: what `seedFindings` (`reviewEvidence.ts`)
 * does to gate output before a fix run's description may carry it.
 *
 * Every pattern here must stay linear, because the body is unbounded text the
 * branch's author controls and it is redacted before it is cut: no nested
 * quantifier, no two quantifiers in a row that can match the same characters,
 * and every quoted value stops at the first matching quote.
 */

/** The most of the gate body a seed carries, counted from its end. */
export const GATE_BODY_MAX_LINES = 200;
export const GATE_BODY_MAX_CHARS = 12_000;

const REDACTED = '[redacted]';
const REDACTED_KEY_BLOCK = '[redacted private key]';

/** From the header through its footer, or to the end of the body when the
 *  footer never came. Matched on the whole body, because a line-at-a-time pass
 *  sees only the header. Covers PKCS#1, PKCS#8, OpenSSH, EC and PGP blocks. */
const PRIVATE_KEY_BLOCK =
  /-----BEGIN [A-Z0-9 ]*PRIVATE KEY[A-Z ]*-----[\s\S]*?(?:-----END [A-Z0-9 ]*PRIVATE KEY[A-Z ]*-----|$)/g;

/** A shell-style assignment at the start of a line: what `env`, `printenv`,
 *  `export -p`, `set` and a `set -x` trace print. Upper-case names only, so an
 *  assertion such as `expected=5` is not mistaken for one. */
const ENV_ASSIGNMENT = /^(\s*(?:\++\s+)?(?:(?:export|set|SET|declare -x)\s+)?[A-Z][A-Z0-9_]*=).*$/;

/**
 * An upper-case key and a colon: YAML, a JSON env dump, and Node's
 * `console.log(process.env)`. A quoted value is always redacted. An unquoted
 * one only under a key with a `_` that does not open with `ERR_`, because
 * `ERROR: Cannot find module`, `TS2345: Argument of type …`, `FAIL: …` and
 * Node's `ERR_MODULE_NOT_FOUND: Cannot find package` are the gate evidence
 * itself — the lines a fix run needs most. `Expected: "foo"` is untouched
 * because the key must be upper-case throughout.
 */
const COLON_ASSIGNMENT =
  /(^|[\s{,(])(["']?)([A-Z][A-Z0-9_]*)\2(\s*:[ \t]*)("[^"]*"|'[^']*'|[^\s,}'"]+)/g;

/** An upper-case name with a `_` assigned inside a line, as a command line
 *  prints it: `Running with STRIPE_KEY=rk_live_… npm test`. The `_` is what
 *  keeps `want FOO=1` in an assertion message whole. */
const MIDLINE_ASSIGNMENT = /(^|[\s'"])([A-Z][A-Z0-9]*_[A-Z0-9_]*=)("[^"]*"|'[^']*'|["']?[^\s'"]*)/g;

const CREDENTIAL_KEYWORD =
  /secret|password|passwd|token|api[_-]?key|access[_-]?key|private[_-]?key/i;

/**
 * A key that names a credential, in any case and spacing, with `=` or `:`:
 * `aws_secret_access_key = …`, `"db_password": "…"`, `apiKey: …`. A value
 * opening with `=` is a comparison (`token === undefined`), not an assignment.
 * The replacer in `redacted` spares two shapes the pattern cannot tell apart.
 *
 * The keyword is found in a lookahead because a lookahead does not backtrack.
 * Spelled `[\w.-]*keyword[\w.-]*`, one long word that repeats a keyword is
 * quadratic: 200k characters of `secretsecret…` took sixteen seconds.
 */
const CREDENTIAL_KEY = new RegExp(
  `(^|[^\\w.-])(["']?(?=[\\w.-]*?(?:${CREDENTIAL_KEYWORD.source}))([\\w.-]+)["']?[ \\t]*([=:])[ \\t]*)("[^"]*"|'[^']*'|\`[^\`]*\`|[^\\s,;'"}=][^\\s,;'"}]*)`,
  'gi',
);

/**
 * `token.ts:12:5` and `api-key.guard.ts:7:1` are a file and a line, and
 * `` expected token: `;` `` is a compiler quoting what it choked on; redacting
 * either takes from a fix run the location it was sent to fix. A dotted key is
 * judged by its last segment rather than by a file extension, because
 * `spring.datasource.password: hunter2` ends in one too. Only a numeric
 * location or a single backtick token may be spared: a grep hit's value can
 * contain another assignment that this regex consumes in the same match.
 */
const SOURCE_LOCATION = /^\d+(?::\d+)*[):]?$/;
const BACKTICK_TOKEN = /^`[^`\s=]*`[):]?$/;

function isGateEvidence(name: string, separator: string, value: string): boolean {
  if (BACKTICK_TOKEN.test(value)) return true;
  return (
    separator === ':' &&
    !CREDENTIAL_KEYWORD.test(name.slice(name.lastIndexOf('.') + 1)) &&
    SOURCE_LOCATION.test(value)
  );
}

/** Each prefix is kept only when it opens a path, so `app/home/page.tsx` in the
 *  repo is left alone while `/home/alice/…` and `file:///home/alice` are not. */
const HOME_DIRECTORIES: readonly RegExp[] = [
  /(^|[^\w.~-])(?:\/var)?\/home\/[^/\\\s'"`:;,()<>[\]{}|]+/g,
  /(^|[^\w.~-])\/Users\/[^/\\\s'"`:;,()<>[\]{}|]+/g,
  /(^|[^\w.~-])\/root(?![\w.-])/g,
  /(^|[^\w.~-])[a-z]:(?:\\+|\/)users(?:\\+|\/)[^/\\\s'"`:;,()<>[\]{}|]+/gi,
];

const CREDENTIALS: readonly [RegExp, string][] = [
  [/\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})/g, REDACTED],
  [/\bglpat-[A-Za-z0-9_-]{20,}/g, REDACTED],
  [/\bsk-[A-Za-z0-9_-]{20,}/g, REDACTED],
  [/\bxox[abprs]-[A-Za-z0-9-]{10,}/g, REDACTED],
  [/\bAKIA[0-9A-Z]{16}\b/g, REDACTED],
  [/\b([Bb]earer\s+)[A-Za-z0-9._~+/-]{8,}=*/g, `$1${REDACTED}`],
  // Not `\b`: that lets a scheme start after every `-` or `.`, and 200k
  // characters of `a-a-…` took fourteen seconds.
  [/(^|[^a-z0-9+.-])([a-z][a-z0-9+.-]*:\/\/)[^/\s:@]+:[^/\s@]*@/gi, `$1$2${REDACTED}@`],
];

/** RFC 1918. A longer dotted number is left whole rather than half-replaced. */
const PRIVATE_IPV4 =
  /(^|[^\w.])(?:10(?:\.\d{1,3}){3}|172\.(?:1[6-9]|2\d|3[01])(?:\.\d{1,3}){2}|192\.168(?:\.\d{1,3}){2})(?!\.?\d)/g;

/**
 * The gate body as a seed may carry it: redacted, then cut to its tail.
 *
 * Gate output is the branch's own test and build output, and the fix run's
 * description — which this body becomes part of — is interpolated by the
 * `s-finalize` prompt that writes a published pull request body. So the shapes
 * that leak a machine or an account are taken out here, before any agent reads
 * them: private key blocks become one `[redacted private key]` line, home
 * directories become `~`, assignment values become `[redacted]` (the shapes are
 * on the patterns above), recognisable tokens and URL credentials become
 * `[redacted]`, and private IPv4 addresses become `[redacted-ip]`. Hostnames,
 * public addresses and secrets in any other shape pass through;
 * OPEN_QUESTIONS.md §22 keeps that list, and is why this is a floor rather
 * than a boundary.
 *
 * Redaction runs before the cap, so the cap can never cut a token down to a
 * suffix no pattern recognises, nor a key block down to headerless lines. The
 * cap keeps the **tail** because test runners print failures and their summary
 * last, and the engine's recorded reason is already a tail of each gate's
 * output. Anything cut is announced on one leading line that does not open with
 * `---`, so `defused` passes it through and the fix run knows it is reading an
 * excerpt.
 */
export function sanitizedGateBody(body: string): string {
  const parts = body
    .replace(PRIVATE_KEY_BLOCK, REDACTED_KEY_BLOCK)
    .split(/(\r\n|\r|\n)/)
    .map((part, i) => (i % 2 === 0 ? redacted(part) : part));
  return tail(parts);
}

function redacted(line: string): string {
  let out = line
    .replace(ENV_ASSIGNMENT, `$1${REDACTED}`)
    .replace(COLON_ASSIGNMENT, (match, lead, quote, key, sep, value: string) =>
      isQuoted(value) || (key.includes('_') && !key.startsWith('ERR_'))
        ? `${lead}${quote}${key}${quote}${sep}${redactedValue(value)}`
        : match,
    )
    .replace(
      MIDLINE_ASSIGNMENT,
      (_, lead, name, value: string) => `${lead}${name}${redactedValue(value)}`,
    )
    .replace(CREDENTIAL_KEY, (match, lead, key, name, separator, value: string) =>
      isGateEvidence(name, separator, value) ? match : `${lead}${key}${redactedValue(value)}`,
    );
  for (const pattern of HOME_DIRECTORIES) out = out.replace(pattern, '$1~');
  for (const [pattern, replacement] of CREDENTIALS) out = out.replace(pattern, replacement);
  return out.replace(PRIVATE_IPV4, '$1[redacted-ip]');
}

function isQuoted(value: string): boolean {
  return value.length >= 2 && (value[0] === '"' || value[0] === "'") && value.endsWith(value[0]);
}

/** A quoted value keeps its quotes, so a redacted JSON or YAML line still
 *  reads as one. */
function redactedValue(value: string): string {
  return isQuoted(value) ? `${value[0]}${REDACTED}${value[0]}` : REDACTED;
}

/** `parts` alternates line, separator, line, … and so always ends on a line. */
function tail(parts: string[]): string {
  let start = parts.length - 1;
  let length = parts[start].length;
  while (start >= 2 && (parts.length - start + 1) / 2 < GATE_BODY_MAX_LINES) {
    const grown = length + parts[start - 1].length + parts[start - 2].length;
    if (grown > GATE_BODY_MAX_CHARS) break;
    length = grown;
    start -= 2;
  }

  const omitted = start / 2;
  const cutsLastLine = length > GATE_BODY_MAX_CHARS;
  if (omitted === 0 && !cutsLastLine) return parts.join('');

  const kept = cutsLastLine
    ? parts[start].slice(-GATE_BODY_MAX_CHARS)
    : parts.slice(start).join('');
  const lines = `${omitted} earlier line${omitted === 1 ? '' : 's'}`;
  const marker = cutsLastLine
    ? `[gate output truncated: ${lines} and the start of the line below omitted]`
    : `[gate output truncated: ${lines} omitted]`;
  return `${marker}\n${kept}`;
}
