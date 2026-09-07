import { ArrowLeft } from 'lucide-react';

import { useNavigation } from '../../context';
import { describeView, parentView } from '../../lib/parentView';

/**
 * The app's one Back control.
 *
 * **It takes no destination.** That is the whole point: five screens used to
 * carry their own Back, each `navigate`-ing somewhere hard-coded, which pushed
 * a new entry rather than popping one — so the stack grew on every "back", the
 * forward stack was destroyed, and the arrow disagreed with the mouse button
 * and `Alt+←` doing the real thing beside it. A control that cannot be told
 * where to go cannot drift from them again.
 *
 * Falls back to `parentView` only when there is no history to pop, which is a
 * deep link or the first screen of a session. See that module for why Up and
 * Back are not the same answer.
 *
 * Disabled rather than hidden when neither is available: a control that comes
 * and goes is harder to rely on than one that greys out, and its absence would
 * reflow the header title beside it on every navigation.
 */
export function BackButton({ className = '' }: { className?: string }) {
  const { view, previousView, canGoBack, goBack, navigate } = useNavigation();

  const up = parentView(view);
  // A root's parent is itself, so there is nowhere to go and nothing to name.
  const target = canGoBack ? previousView : (up === view ? null : up);

  return (
    <button
      type="button"
      data-testid="back-button"
      aria-label="Back"
      title={target ? `Back to ${describeView(target)}` : 'Back'}
      disabled={target === null}
      onClick={() => (canGoBack ? goBack() : navigate(up, 'replace'))}
      className={`btn-icon shrink-0 disabled:cursor-not-allowed disabled:opacity-30 ${className}`}
    >
      <ArrowLeft className="w-4 h-4" aria-hidden="true" />
    </button>
  );
}

export default BackButton;
