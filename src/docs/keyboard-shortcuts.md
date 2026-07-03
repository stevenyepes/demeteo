# Keyboard Shortcuts

Demeteo provides a full set of keyboard shortcuts for power users.

| Shortcut | Action |
|----------|--------|
| `Cmd/Ctrl + K` | Open Command Palette |
| `Cmd/Ctrl + T` | New Feature (inside a project) |
| `Esc` | Close modal / pop navigation |
| `?` | Show this shortcut reference |
| `Cmd/Ctrl + Shift + N` | New Feature (deprecated alias for `Cmd/Ctrl + T`) |
| `Cmd/Ctrl + N` | New Project |
| `Cmd/Ctrl + W` | Close current modal / pop navigation |
| `Cmd/Ctrl + ,` | Open Settings |
| `Cmd/Ctrl + 1-9` | Switch to project #1-9 |
| `Cmd/Ctrl + B` | Toggle sidebar |
| `Cmd/Ctrl + Shift + F` | Command palette |
| `Cmd/Ctrl + .` | Focus command palette (alternative) |
| `Cmd/Ctrl + G` | Next feature |
| `Cmd/Ctrl + Shift + G` | Previous feature |
| `Cmd/Ctrl + ?` | Open documentation |
| `F1` | Open documentation |

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
