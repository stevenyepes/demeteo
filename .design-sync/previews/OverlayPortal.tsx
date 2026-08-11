import { OverlayPortal } from 'demeteo';

/**
 * OverlayPortal renders its children into `document.body` instead of in
 * place, so a `fixed inset-0` overlay escapes any ancestor stacking context.
 * The `container` prop overrides the target — which is what makes the escape
 * visible in a preview: rendering into a local element shows the *in-place*
 * result next to the portalled one.
 */

/** Portalled into `document.body`: the panel escapes the bordered box that
 *  lexically contains it and lands at the top-left of the card. */
export const EscapesItsParent = () => (
  <div className="relative z-0 w-full max-w-md p-5 rounded-xl bg-[#0d0f14] border border-violet-500/30">
    <div className="text-xs font-mono text-violet-300 uppercase tracking-wider mb-2">
      stacking context (position: relative, z-0)
    </div>
    <p className="text-sm text-slate-400">
      The portalled panel below is a child of this box in the React tree, but
      not in the DOM — look for it at the top of the card, outside this border.
    </p>
    <OverlayPortal>
      <div className="fixed top-4 right-4 px-4 py-3 rounded-lg bg-violet-500/15 border border-violet-400/50 backdrop-blur-sm">
        <span className="text-xs font-mono text-violet-200">portalled to document.body</span>
      </div>
    </OverlayPortal>
  </div>
);

/** `container={null}` renders nothing — the escape hatch tests use. */
export const NullContainerRendersNothing = () => (
  <div className="w-full max-w-md p-5 rounded-xl bg-[#0d0f14] border border-white/5">
    <p className="text-sm text-slate-400">
      With <code className="font-mono text-slate-300">container={'{null}'}</code> there is no
      target, so the portal renders nothing at all — no crash, no stray node.
    </p>
    <OverlayPortal container={null}>
      <div className="px-4 py-3 rounded-lg bg-ruby-500/15 border border-ruby-500/40">
        <span className="text-xs font-mono text-ruby-200">you should never see this</span>
      </div>
    </OverlayPortal>
  </div>
);
