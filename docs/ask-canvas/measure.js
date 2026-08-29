const sel = ['.transcript', '.empty-wrap', '.well', '.insp-body', '.panel', '.sheet', '.app', '.frame'];
const out = [];
for (const s of sel) {
  document.querySelectorAll(s).forEach((el, i) => {
    out.push(`${s}[${i}] client=${el.clientHeight} scroll=${el.scrollHeight} overflow=${el.scrollHeight - el.clientHeight}`);
  });
}
// last child bottom inside each transcript, to catch clipped bubbles
document.querySelectorAll('.transcript').forEach((t) => {
  const r = t.getBoundingClientRect();
  const kids = [...t.children];
  const last = kids[kids.length - 1].getBoundingClientRect();
  out.push(`transcript bottom slack = ${Math.round(r.bottom - last.bottom)}px (top slack ${Math.round(kids[0].getBoundingClientRect().top - r.top)})`);
});
out.join('\n');
