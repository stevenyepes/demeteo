import { readFileSync, writeFileSync, existsSync, mkdirSync } from 'node:fs';
const inc = (s) => s.replace(/<!--@ *([\w./-]+) *-->/g, (_, f) => inc(readFileSync(f, 'utf8')));
mkdirSync('probe', { recursive: true });
for (const n of process.argv.slice(2)) {
  const css =
    readFileSync('_tokens.css', 'utf8') +
    (existsSync(`css/${n}.css`) ? readFileSync(`css/${n}.css`, 'utf8') : '');
  writeFileSync(
    `probe/${n}.html`,
    `<!doctype html><html><head><meta charset="utf-8">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Outfit:wght@500;600;700;800&family=Inter:wght@400;500;600&family=Fira+Code:wght@400;500&display=swap">
<style>${css}</style></head><body style="margin:0">${inc(readFileSync(`body/${n}.html`, 'utf8'))}</body></html>`,
  );
}
console.log('probes written');
