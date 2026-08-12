/**
 * Which density the top header bar renders its nav entries at
 * (UI_REDESIGN_PLAN §5.1 — a rule that decides what should happen does not
 * live inside the component that renders it).
 *
 * **Measure the header, not the nav cluster.** The observed element spans the
 * window and does *not* change width when labels are dropped, so unlike
 * `headerCollapse` there is no feedback loop here and the band is not
 * load-bearing against one: it damps jitter across a resize drag, and it is
 * what stops a later edit from measuring the cluster instead. Measuring the
 * cluster reopens exactly the loop `headerCollapse` documents — labels hidden →
 * cluster narrows → labels fit → labels shown.
 *
 * **Why measure at all, given the labels were never hidden.** The
 * `hidden md:inline` this replaces fired at every reachable size: `md:` is
 * `min-width: 768px`, `src-tauri/tauri.conf.json` pins the window to a 1024px
 * floor, and 1024 ≥ 768 — so the live half was `md:inline` and the dead half
 * was the `hidden` base. Nothing about the old markup was broken; it answered
 * the wrong question. A viewport query reports how wide the *window* is, and
 * the thing that runs out of room is the header's side track, which is the
 * window minus the padding, the gaps and whatever the centre track claims. The
 * two only agree while the centre track is fixed-width, which is exactly what
 * this pass stopped being true.
 *
 * **Two tiers, both reachable.** A third `overflow` tier would be dead code the
 * day it landed: at the 1024px floor the icon-only cluster measures 232px
 * against a 349px side track, so it never overflows anywhere the window can go.
 *
 * **The numbers are measured, not estimated.** Laid out in WebKitGTK — the
 * engine the Linux desktop build runs on — at `px-6` padding (48px) and two
 * `gap-4` inter-track gaps (32px), a side track is `0.38·W − 40` while the
 * centre clamp `clamp(13rem,24vw,28rem)` is on its `24vw` arm, and
 * `(W − 528) / 2` once it saturates at 1867. The right cluster measures
 * **485px labelled** and **232px icon-only** — the labelled figure is 45px
 * above the glyph-count estimate this shipped with, and that gap is why these
 * constants are not the spec's 1344/1440. Labels fit from **1382px** up.
 *
 * Every figure above equates `W`, the *measured* `offsetWidth` this function is
 * handed, with `100vw` — the clamp's `24vw` arm resolves against the viewport,
 * not against the element. That holds only because `src/App.tsx` roots the app
 * in `w-screen` + `overflow-hidden`, so the header spans the full viewport with
 * no scrollbar gutter taken out of it. Inset the header, wrap it in a padded
 * container, or let a root scrollbar appear, and the centre track stops being
 * `0.24·W`: every threshold here is then wrong, and nothing in the tree detects
 * it.
 *
 * **The centre clamp is what buys the default window its labels.** At the
 * `28vw` this pass was first written with, a side track is `0.36·W − 40` =
 * 478px at the 1440 default against a 485px labelled cluster: the four nav
 * entries lose their names on the window the app opens at, which they have
 * never done. `24vw` leaves the search 346px at that width, and 22px of clearance
 * around the cluster; how that compares to the fixed `w-64` it replaced is
 * itself width-dependent, and UI_REDESIGN_PLAN §5.1 carries the crossover. Tightening the header to `gap-2` was measured too and is not
 * enough on its own: 1.5px at 1440, i.e. a collision one font metric away.
 *
 * What a wrong figure costs is bounded in the markup rather than here: the nav
 * grid track carries a `min-content` floor (`src/components/TopBar.tsx`), so a
 * cluster that outgrows its share moves the search box off centre instead of
 * drawing over it. That is a floor under the damage, not a reason to skip the
 * re-measure.
 *
 * A band whose *lower* edge sits under the fit point is not a jitter damper, it
 * is a licence to overlap: the band holds `labels` across a whole resize drag,
 * so anywhere inside it the labelled cluster has to fit. The icons threshold
 * therefore sits above 1382 rather than at it, and the band's upper edge is the
 * default window width itself.
 *
 * Re-measure both cluster widths before touching the gaps, the clamp, or the
 * number of nav entries — every one of them moves the fit point, and the
 * arithmetic above under-reports the labelled cluster by 45px on its own.
 *
 * `width <= 0` returning `current` is not defensive: jsdom lays nothing out and
 * `offsetWidth` is 0 before the first paint, so without that arm every test and
 * every first frame would collapse to `icons`.
 */

export type HeaderDensity = 'labels' | 'icons';

/** Below this measured header width, nav entries render icon-only. */
export const HEADER_ICONS_BELOW_PX = 1392;
/** At or above this measured header width, nav entries render their labels.
 *  This is `src-tauri/tauri.conf.json`'s default window width: the window the
 *  app opens at renders labelled. */
export const HEADER_LABELS_AT_PX = 1440;

export function nextHeaderDensity(width: number, current: HeaderDensity): HeaderDensity {
  if (width <= 0) return current;
  if (width >= HEADER_LABELS_AT_PX) return 'labels';
  if (width < HEADER_ICONS_BELOW_PX) return 'icons';
  return current;
}
