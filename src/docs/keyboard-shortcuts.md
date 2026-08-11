# Keyboard Shortcuts

Demeteo provides a full set of keyboard shortcuts for power users.

This page is written by hand. `src/lib/shortcuts.ts` is what the app and the in-app
help overlay (`F1`) actually bind — if the two ever disagree, believe the overlay.

## Global

| Shortcut | Action |
|----------|--------|
| `Cmd/Ctrl + K` | Open Command Palette |
| `Cmd/Ctrl + P` | Open Command Palette (alias) |
| `Cmd/Ctrl + T` | New Feature (inside a project) |
| `Cmd/Ctrl + Shift + N` | New Feature (deprecated alias for `Cmd/Ctrl + T`) |
| `Cmd/Ctrl + N` | New Project |
| `Cmd/Ctrl + 1-9` | Switch to project #1-9 |
| `Cmd/Ctrl + G` | Next feature |
| `Cmd/Ctrl + Shift + G` | Previous feature |
| `Cmd/Ctrl + W` | Close current modal / pop navigation |
| `Cmd/Ctrl + ,` | Open Settings |
| `Cmd/Ctrl + B` | Toggle sidebar |
| `Cmd/Ctrl + \`` | Open the Terminals view |
| `Cmd/Ctrl + Shift + \`` | New terminal (from the Terminals view) |
| `Cmd/Ctrl + R` | Reload data for the current view |
| `Alt + Left` | Navigate back |
| `Alt + Right` | Navigate forward |
| `Esc` | Close modal / pop navigation |
| `F1` | Show this shortcut reference |
| `?` | Show this shortcut reference |
| `F11` | Toggle fullscreen |

`Cmd/Ctrl + Shift + T` is deliberately left unbound so the webview keeps its
reopen-closed-tab behaviour.

## Run view

These are single keys with no modifier, and they are live **only** while a feature's
run view is open and focus is outside a text field. They do nothing elsewhere in the
app. Holding Cmd/Ctrl turns them back into the global shortcuts above — `G` and `T`
mean different things bare and modified.

| Shortcut | Action |
|----------|--------|
| `J` | Select the next step in the run |
| `K` | Select the previous step in the run |
| `Enter` | Move focus into the step inspector |
| `G` | Show the run as the workflow graph |
| `T` | Show the run as the step timeline |

## Mouse

| Gesture | Action |
|---------|--------|
| `XButton1` (mouse back) | Navigate back |
| `XButton2` (mouse forward) | Navigate forward |

The XButton1 and XButton2 gestures wired by `MouseNavigationBridge` integrate with the same in-app navigation stack as the keyboard shortcuts above — the back/forward history is shared across both input modes.

## Tips

- The **Command Palette** (`Cmd/Ctrl + K`) is the fastest way to navigate. Start typing to fuzzy-match projects, features, workflows, and settings.
- `Esc` closes any open modal, drawer, popover, or the command palette; with nothing open it pops one entry off the in-app navigation history.
- Use `Cmd/Ctrl + 1` through `Cmd/Ctrl + 9` to jump directly to projects by their order in the sidebar.
- `Cmd/Ctrl + T` starts a new feature inside the current project and is a no-op when no project is selected.
- On a long run, `J`/`K` plus `Enter` walks the whole pipeline without touching the mouse.
