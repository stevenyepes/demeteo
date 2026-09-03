## Reproduction Case

Committed as `src/components/discovery/DiscoveryView.editing.test.tsx`:

```tsx
describe('switching tickets while one is open for edit', () => {
  it('does not keep the editor open on the ticket that is no longer selected', async () => {
    const view = render(<DiscoveryView discoveryId="d-1" discoveryTitle="multi-client runner" />);
    await view.findByPlaceholderText(/./);

    fireEvent.click(view.getByRole('radio', { name: 'Board' }));

    const boardEl = await view.findByTestId('ticket-board');
    const firstCard = within(boardEl).getByText('First ticket').closest('button');
    if (!firstCard) throw new Error('first ticket card did not render');
    fireEvent.click(firstCard);

    fireEvent.click(await view.findByTestId('ticket-edit'));
    const editor = await view.findByTestId('ticket-editor');
    expect(within(editor).getByLabelText('Title')).toHaveValue('First ticket');

    const secondCard = within(boardEl).getByText('Second ticket').closest('button');
    if (!secondCard) throw new Error('second ticket card did not render');
    fireEvent.click(secondCard);

    await waitFor(() => {
      const editorStillOpen = view.queryByTestId('ticket-editor');
      if (editorStillOpen) {
        expect(within(editorStillOpen).getByLabelText('Title')).not.toHaveValue('First ticket');
      }
    });
  });
});
```

Run it with:

```bash
npx vitest run src/components/discovery/DiscoveryView.editing.test.tsx
```

It opens a Discovery workspace with two generated tickets, switches to the Board
view, opens the editor for "First ticket", then clicks "Second ticket"'s card.
The `waitFor` times out because `ticket-editor` still renders with the Title
input holding `"First ticket"` — the editor never closes or switches, exactly
matching the report ("the edit ticket view keeps open" when clicking another
ticket while editing, on a discovery whose tickets are already generated).

## Execution Trace

1. `DiscoveryView` (`src/components/discovery/DiscoveryView.tsx`) owns two
   independent pieces of state: `selectedId` (`:80`) and `editingId` (`:90`).
2. `onEdit={() => setEditingId(selected.ticket.id)}` (`:405`) is wired from
   `TicketInspector`'s Edit button, opening `TicketEditorDrawer` for that
   ticket.
3. The render branch at `:371-409` is an unconditional priority: `editing ?
   <TicketEditorDrawer .../> : selected && <TicketInspector .../>`. As long as
   `editingId` is non-null, the drawer renders — regardless of `selectedId`.
4. The ticket list/board/graph only ever calls `onSelect={setSelectedId}`
   (`:368`, `TicketColumn` → `TicketBoard.tsx:67` /
   `TicketGraph.tsx` → `TicketBoardCard.tsx:59` /
   `TicketGraphNode.tsx:67`). Clicking a different ticket's card updates
   `selectedId` only.
5. `editing = editingId !== null ? index.get(editingId) : undefined`
   (`:294`) is derived solely from `editingId`, which nothing in the
   selection path touches.
6. Result: after clicking another ticket, `selected` changes but `editing`
   does not, so the ternary at `:371` keeps taking the `editing` branch and
   `TicketEditorDrawer` keeps rendering the original ticket's data. The only
   way to leave edit mode is the drawer's own `onClose` (`:380`,
   `() => setEditingId(null)`), which the user never clicked.

## Root Cause

`editingId` and `selectedId` in `DiscoveryView.tsx` are two independent,
uncoordinated `useState` values. Selecting a different ticket (via the board
or graph card click, which only calls `setSelectedId`) never clears or
repoints `editingId`, and the render logic at `DiscoveryView.tsx:371`
unconditionally prefers `editing` over `selected` whenever `editingId` is
non-null. This is the assumption that's violated: the code assumes a ticket
selection change implies leaving edit mode, but no code path actually enforces
that — the two states drift apart the moment the user clicks a sibling ticket
while editing.

## Fix Boundary

**In scope:**
- `src/components/discovery/DiscoveryView.tsx` — the `onSelect` handler
  passed to `TicketColumn` (`:368`) and/or the `editing`/`selected` render
  branch (`:371-409`) need to keep `editingId` and `selectedId` coherent (e.g.
  clear `editingId` on selecting a different ticket, or key the drawer's
  visibility on `editing.ticket.id === selectedId`).

**Must not change:**
- `TicketEditorDrawer.tsx`, `TicketInspector.tsx` — their props/contracts are
  fine; the drawer is correctly keyed by `editing.ticket.id` and remounts
  cleanly once fed the right ticket.
- `TicketBoard.tsx`, `TicketBoardCard.tsx`, `TicketGraph.tsx`,
  `TicketGraphNode.tsx`, `TicketColumn.tsx` — selection plumbing is correct;
  the bug is entirely in how the parent reconciles `editingId` against a
  `selectedId` change, not in how selection events are dispatched.
- Backend (`ticket_update`, discovery board derivation) — this is a pure
  frontend state-coordination bug; the board data itself is correct.

## Risk

Low. The fix is confined to how `DiscoveryView` reconciles two pieces of
local UI state; it doesn't touch data fetching, persistence, or the
`TicketEditorDrawer`'s own save/dirty logic. The one behavioral choice to get
right is UX intent — whether switching tickets while editing should silently
discard the in-progress edit (matching "Discard" button semantics already
in the drawer) or prompt/carry the draft forward — but either choice is
containable inside `DiscoveryView.tsx` and doesn't risk regressing unrelated
ticket actions (start, force-start, drop, decompose) since those read
`editing`/`selected` independently and are untouched by this fix.
