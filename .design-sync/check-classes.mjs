#!/usr/bin/env node
/*
 * Guard against the one failure mode that is invisible in this repo:
 * a utility class used in a preview that does not exist in the compiled CSS.
 *
 * Tailwind v4 scans only what `src/App.css`'s `@source` names — the `src/`
 * tree. `.design-sync/previews/` is outside it, so a class the app happens
 * never to use (`bg-violet-500/80` is the one that caught us) produces no
 * rule at all. Nothing errors: the element simply renders unstyled, which on
 * a dark surface reads as "slightly off" rather than "broken", and can
 * survive a screenshot review.
 *
 * Usage: node .design-sync/check-classes.mjs
 * Exits 1 and lists every unresolved token. Suggests the nearest existing
 * opacity/shade sibling, since that is almost always the intended class.
 */

import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const CSS = '.design-sync/.cache/app.css';
const DIR = '.design-sync/previews';

const css = readFileSync(CSS, 'utf8');

// Tailwind escapes any non-ident char in the selector with a backslash.
const escape = (t) => t.replace(/[^A-Za-z0-9_-]/g, (ch) => '\\' + ch);
const present = (t) => css.includes('.' + escape(t));

// Only literal className strings — template interpolation is out of reach
// and is deliberately avoided in the authored previews.
const CLASS_RE = /className\s*=\s*"([^"]+)"/g;

const missing = new Map();
for (const f of readdirSync(DIR).filter((f) => f.endsWith('.tsx'))) {
  const src = readFileSync(join(DIR, f), 'utf8');
  for (const m of src.matchAll(CLASS_RE)) {
    for (const tok of m[1].split(/\s+/).filter(Boolean)) {
      if (present(tok)) continue;
      if (!missing.has(tok)) missing.set(tok, new Set());
      missing.get(tok).add(f);
    }
  }
}

if (!missing.size) {
  console.log('✓ every preview class resolves in the compiled CSS');
  process.exit(0);
}

// A near-miss is usually the same utility at a different opacity or shade.
const stem = (t) => t.split('/')[0].replace(/-\d+$/, '');
const siblings = (t) => {
  const s = stem(t);
  const re = new RegExp('\\.' + escape(s) + '[-\\\\/][^{,\\s:]*', 'g');
  return [...new Set((css.match(re) ?? []).map((x) => x.slice(1).replace(/\\/g, '')))].slice(0, 6);
};

console.error(`✗ ${missing.size} preview class(es) have no rule in ${CSS}:\n`);
for (const [tok, files] of [...missing].sort()) {
  console.error(`  ${tok}`);
  console.error(`    used in: ${[...files].join(', ')}`);
  const alt = siblings(tok);
  if (alt.length) console.error(`    existing: ${alt.join('  ')}`);
}
console.error('\nEither use an existing class or add the utility to a component under src/.');
process.exit(1);
