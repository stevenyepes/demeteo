import type { AppView } from '../types';

/**
 * Where a view sits in the hierarchy, and what to call the place Back leads to.
 *
 * Two different questions, kept in one file because they answer the same
 * control. Both are pure and synchronous, so the Back button's behaviour is
 * reachable from a test without mounting a provider.
 *
 * **Up is not Back.** Back pops history — wherever the user actually came from,
 * which is the only honest answer when there is one. `parentView` is the
 * fallback for when there is no history at all: a deep link, a notification, a
 * run opened as the first thing this session. Using it in place of a pop is the
 * bug this module exists to end — five screens each hard-coded their own
 * destination and pushed it, so Back grew the stack it was supposed to shrink
 * and the visible arrow disagreed with the mouse button beside it.
 */
export function parentView(view: AppView): AppView {
  switch (view.kind) {
    // Roots. Nothing is above them, so Back is disabled rather than pointed
    // somewhere arbitrary.
    case 'empty-state':
    case 'home':
      return view;

    case 'editor':
      // The editor is opened from a feature and from a sync review; both carry
      // the feature. Without one it was reached from the project.
      return view.featureId
        ? { kind: 'detail', featureId: view.featureId, featureTitle: view.featureTitle ?? '' }
        : { kind: 'home' };

    case 'workflow-editor':
      return { kind: 'workflows' };

    // A Discovery is opened from its own tab, so Up returns to that tab rather
    // than to the default one. Ask has no resting tab of its own — choosing it
    // navigates here instead of swapping a panel — so it goes up to Project
    // Home unqualified.
    case 'discovery':
      return { kind: 'home', section: 'discovery' };

    default:
      return { kind: 'home' };
  }
}

/**
 * A short name for a view, for the Back control's tooltip.
 *
 * A bare arrow is only safe when its destination is discoverable, and hover is
 * where that is cheap — spelling it on the button costs header width on every
 * screen. Names the place, not the action: the `aria-label` already says
 * "Back".
 */
export function describeView(view: AppView): string {
  switch (view.kind) {
    case 'empty-state':
      return 'the workspace';
    case 'home':
      return 'the project';
    case 'detail':
      return view.featureTitle || 'the feature';
    case 'editor':
      return 'the code';
    case 'new-project':
      return 'the new project';
    case 'create-project':
      return 'the project wizard';
    case 'project-settings':
      return 'project settings';
    case 'code-review':
      return 'code review';
    case 'workflows':
      return 'workflows';
    case 'workflow-editor':
      return 'the workflow editor';
    case 'discovery':
      return view.discoveryTitle || 'the discovery';
    case 'ask':
      return 'Ask';
    case 'providers':
      return 'providers';
    case 'settings':
      return 'settings';
    case 'remote-inbox':
      return 'runs';
    case 'terminals':
      return 'terminals';
  }
}
