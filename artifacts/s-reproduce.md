## Reproduction Case

The failing test is `src/lib/attachments.test.ts` in the `extractImageFilesFromClipboard` suite:

```ts
it("returns a macOS clipboard image exposed as TIFF", () => {
  const tiff = file("clipboard.tiff", "image/tiff");
  const dt = makeClipboardData([
    { kind: "file", type: "image/tiff", getAsFile: () => tiff },
  ]);

  expect(extractImageFilesFromClipboard(dt)).toEqual([tiff]);
});
```

Run it with:

```bash
npx vitest run src/lib/attachments.test.ts
```

This reproduces the failure using the browser `DataTransfer` shape observed for a macOS image paste: the item is a file with MIME `image/tiff`. The current implementation returns `[]`, so the assertion fails. In this fenced checkout the command could not start because dependencies are not installed (`vitest` and `@vitejs/plugin-react` are unresolved); the test is committed without a production fix and fails once the declared dependencies are available.

## Execution Trace

1. The user copies an image on macOS and presses Cmd+V in an attachment surface.
2. For the inline ProjectHome composer, the browser dispatches `paste` to the composer container at `src/components/ProjectHome.tsx:90`; for the full attachment surface, it dispatches to `AttachmentDropzone` at `src/components/AttachmentDropzone.tsx:225`.
3. The handler calls `extractImageFilesFromClipboard(e.clipboardData)` at `src/components/ProjectHome.tsx:99` or `src/components/AttachmentDropzone.tsx:227`.
4. `src/lib/attachments.ts:264` iterates `clipboardData.items`. At `src/lib/attachments.ts:271-272`, it requires `kind === "file"` and membership in the frontend MIME allow-list.
5. `image/tiff` is absent from the allow-list at `src/lib/attachments.ts:241-246`, so `getAsFile()` is never called and the helper returns `[]` at `src/lib/attachments.ts:276`.
6. Because the result is empty, the component returns before `preventDefault()` and before `stageClipboardFile`/`ingestFiles`. No preview, staging entry, or attachment IPC call is produced.
7. Even if the frontend filter were bypassed, the backend validator at `crates/demeteo-core/src/application/attachments.rs:221-227` also rejects `image/tiff`, so the eventual attachment commit would fail unless the backend allow-list is updated consistently.

## Root Cause

The attachment pipeline assumes that every clipboard image will be represented by one of four web image MIME types (`image/png`, `image/jpeg`, `image/gif`, or `image/webp`). macOS image paste can instead expose the clipboard item as `image/tiff`; the frontend allow-list therefore drops the item before `getAsFile()`, event cancellation, preview generation, and staging, and the matching Rust allow-list would reject TIFF later as well. This is an incomplete MIME allow-list, not a Cmd/Ctrl event-modifier problem.

## Fix Boundary

Files in scope for the downstream fix:

- `src/lib/attachments.ts` — add the supported macOS clipboard MIME representation to the shared frontend filter.
- `crates/demeteo-core/src/application/attachments.rs` — keep backend attachment validation aligned with the frontend allow-list.
- `src/lib/attachments.test.ts` — retain the committed TIFF reproduction and add/adjust allow-list coverage as needed.
- Relevant existing component/backend attachment tests only if required to verify the two paths; do not change unrelated paste or keyboard behavior.

Files that must not change for this bug: `ProjectHome.tsx`, `AttachmentDropzone.tsx`, `StartFeatureModal.tsx`, Tauri capabilities, agent spawn logic, worktree/merge code, database migrations, or attachment storage semantics. The current event routing and staging flow should remain unchanged; only MIME recognition/validation should be corrected.

## Risk

The contained fix has low regression risk because it broadens an existing image allow-list and leaves event routing, hashing, staging, IPC, and storage unchanged. The main compatibility risk is accepting a TIFF that downstream preview or agent tooling cannot consume; that is why the frontend and Rust validators must be changed together and covered by tests. Unsupported image formats such as BMP and SVG should remain rejected.
