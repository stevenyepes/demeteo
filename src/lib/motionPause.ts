/**
 * Pauses every CSS animation while the window is unfocused or hidden, by
 * setting `data-motion="paused"` on `<html>`; the rule that reads it sits
 * beside the animations in `src/App.css`.
 *
 * On WebKitGTK any running animation — opacity-only included — makes the
 * webview produce a frame per display refresh, and the GTK3 UI thread pays
 * several milliseconds for each one. A single pulsing badge held demeteo at
 * ~35-90% of a core plus a share of the GPU for as long as it sat in the
 * background, which is precisely when a pending gate leaves it. Compositor
 * hints (`will-change`, dropping the glow's shadow) changed nothing; only a
 * stopped or paused animation stops the frames.
 *
 * Unfocused, not only hidden: a window moved to another Hyprland workspace
 * stays `visible` to the page, so `document.hidden` alone never fires there.
 * The cost is that a window left in view beside a focused one shows its
 * spinners frozen until it is focused again.
 */
export function installMotionPause(win: Window = window): () => void {
  const doc = win.document;
  const sync = () => {
    if (doc.hidden || !doc.hasFocus()) {
      doc.documentElement.dataset.motion = 'paused';
    } else {
      delete doc.documentElement.dataset.motion;
    }
  };

  sync();
  win.addEventListener('focus', sync);
  win.addEventListener('blur', sync);
  doc.addEventListener('visibilitychange', sync);
  return () => {
    win.removeEventListener('focus', sync);
    win.removeEventListener('blur', sync);
    doc.removeEventListener('visibilitychange', sync);
    delete doc.documentElement.dataset.motion;
  };
}
