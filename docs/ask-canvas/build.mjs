import { readFileSync, writeFileSync, existsSync } from 'node:fs';

const inc = (s) => s.replace(/<!--@ *([\w./-]+) *-->/g, (_, f) => inc(readFileSync(f, 'utf8')));

for (const n of process.argv.slice(2)) {
  const css =
    readFileSync('_tokens.css', 'utf8') +
    (existsSync(`css/${n}.css`) ? readFileSync(`css/${n}.css`, 'utf8') : '');
  const out =
    readFileSync('_head_a.html', 'utf8') +
    css +
    readFileSync('_head_b.html', 'utf8') +
    inc(readFileSync(`body/${n}.html`, 'utf8')) +
    readFileSync('_tail.html', 'utf8');
  writeFileSync(`${n}.dc.html`, out);
  console.log(`built ${n}.dc.html — ${out.length} bytes`);
}
