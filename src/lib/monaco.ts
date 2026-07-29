/** Options every Monaco mount in the app must spread in.
 *
 *  Monaco measures its container once, at construction, and never again
 *  unless `automaticLayout` is set — it installs a ResizeObserver only for
 *  that flag. Every editor here lives inside a box whose height is settled
 *  *after* mount: a modal that fades in, the artifact panel that slides open,
 *  a window the user resizes. Without this the editor keeps the size it was
 *  born with (often a few pixels, or zero) and renders as a blank sliver
 *  with no scrollbars — content is there, laid out against stale dimensions.
 *
 *  There is no `height`/`width` here on purpose: sizing stays with the
 *  surrounding flex layout, which needs `min-h-0` on the chain down to the
 *  editor's host so `height="100%"` resolves against something definite.
 */
export const MONACO_RESIZE_SAFE = {
  automaticLayout: true,
} as const;
