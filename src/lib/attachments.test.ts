// Tests for the attachment staging seam.
//
// The bug this guards: Tauri v2 drag-and-drop hands the webview an absolute
// PATH, not a browser `File`. The dropzone used to stage those with `size = 0`
// and a fabricated random sha ("staged-<uuid>"), so the chip read "0 B" and the
// sha-based dedup let the same file through twice — two chips, two commits.
//
// The fix routes path-based drops through `stageAttachmentMetadata`, which
// reads the bytes in Rust and returns the real `{ sha256, size }`, making the
// dedup key stable across re-drops.
//
// Was `tests/repro/attachment-dnd-staging.mjs`, which re-implemented the JS
// ingest logic AND the Rust command in Node, then asserted the copies agreed
// with each other. This imports the real wrappers; the Rust sha256 itself is
// covered by `crates/demeteo-core/tests/domain/attachment.rs`.

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  computeLocalSha256,
  extractClipboardImageFiles,
  extractImageFilesFromClipboard,
  stageAttachmentMetadata,
} from "./attachments";

// Known vector: sha256("hello world"). The Rust side must agree byte-for-byte,
// since the two shas are used as the same dedup key.
const HELLO_WORLD_SHA = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});

describe("computeLocalSha256", () => {
  it("produces lowercase hex matching the canonical SHA-256", async () => {
    const file = new File(["hello world"], "note.txt", { type: "text/plain" });

    expect(await computeLocalSha256(file)).toBe(HELLO_WORLD_SHA);
  });

  it("gives the same file the same digest across two reads (a stable dedup key)", async () => {
    const first = new File(["same bytes"], "a.txt");
    const second = new File(["same bytes"], "b.txt");

    // Dedup keys on content, not filename — re-dropping one file must collide.
    expect(await computeLocalSha256(first)).toBe(await computeLocalSha256(second));
  });
});

describe("stageAttachmentMetadata", () => {
  it("routes a dropped path to the Rust command with an explicit null mime", async () => {
    vi.mocked(invoke).mockResolvedValue({ sha256: HELLO_WORLD_SHA, size: 11 });

    const meta = await stageAttachmentMetadata("/tmp/note.txt");

    expect(meta).toEqual({ sha256: HELLO_WORLD_SHA, size: 11 });
    expect(invoke).toHaveBeenCalledWith("attachment_stage_metadata", {
      sourcePath: "/tmp/note.txt",
      mime: null,
      sourceFilename: null,
    });
  });

  it("returns the real byte count so the chip never renders 0 B", async () => {
    vi.mocked(invoke).mockResolvedValue({ sha256: HELLO_WORLD_SHA, size: 4096 });

    const meta = await stageAttachmentMetadata("/tmp/big.pdf", "application/pdf", "big.pdf");

    expect(meta.size).toBe(4096);
    expect(meta.sha256).not.toMatch(/^staged-/);
  });

  it("propagates a Rust validation error instead of staging the entry", async () => {
    vi.mocked(invoke).mockRejectedValue({ kind: "validation", message: "file too large" });

    await expect(stageAttachmentMetadata("/tmp/huge.bin")).rejects.toMatchObject({
      kind: "validation",
    });
  });
});

describe("re-drop dedup", () => {
  it("yields one identical sha for the same path dropped twice", async () => {
    vi.mocked(invoke).mockResolvedValue({ sha256: HELLO_WORLD_SHA, size: 11 });

    const first = await stageAttachmentMetadata("/tmp/note.txt");
    const second = await stageAttachmentMetadata("/tmp/note.txt");

    // The dropzone dedups with `filter((e) => e.sha256 !== sha256)`, so a
    // stable sha is what collapses the second drop onto the first. The old
    // random "staged-<uuid>" sha is exactly why that filter used to miss.
    expect(second.sha256).toBe(first.sha256);
  });
});

// Local clipboard fixture: jsdom ships no usable `DataTransfer` constructor
// for paste events, so each test builds a `{ items }` object structurally
// typed to satisfy the subset of `DataTransferItemList` that
// `extractImageFilesFromClipboard` actually reads (`length` + indexed
// access). Cast through `unknown` to keep the helper's public `DataTransfer`
// signature honest without importing a stub from the global test setup.
interface FakeClipboardItem {
  kind: string;
  type: string;
  getAsFile: () => File | null;
}

function makeClipboardData(items: FakeClipboardItem[]): DataTransfer {
  return { items } as unknown as DataTransfer;
}

function file(name: string, type: string, bytes: string = "x"): File {
  return new File([bytes], name, { type });
}

describe("extractImageFilesFromClipboard", () => {
  it("returns an empty array when items is empty", () => {
    expect(extractImageFilesFromClipboard(makeClipboardData([]))).toEqual([]);
  });

  it("ignores text and html items (kind === 'string')", () => {
    const stringSpy = vi.fn(() => null);
    const htmlSpy = vi.fn(() => null);
    const dt = makeClipboardData([
      { kind: "string", type: "text/plain", getAsFile: stringSpy },
      { kind: "string", type: "text/html", getAsFile: htmlSpy },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([]);
    expect(stringSpy).not.toHaveBeenCalled();
    expect(htmlSpy).not.toHaveBeenCalled();
  });

  it("ignores unsupported image MIME types (BMP and SVG)", () => {
    const bmp = file("a.bmp", "image/bmp");
    const svg = file("a.svg", "image/svg+xml");
    const bmpSpy = vi.fn(() => bmp);
    const svgSpy = vi.fn(() => svg);
    const dt = makeClipboardData([
      { kind: "file", type: "image/bmp", getAsFile: bmpSpy },
      { kind: "file", type: "image/svg+xml", getAsFile: svgSpy },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([]);
    expect(bmpSpy).not.toHaveBeenCalled();
    expect(svgSpy).not.toHaveBeenCalled();
  });

  it("returns a single supported image file", () => {
    const png = file("a.png", "image/png");
    const dt = makeClipboardData([
      { kind: "file", type: "image/png", getAsFile: () => png },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([png]);
  });

  it("returns a macOS clipboard image exposed as TIFF", () => {
    const tiff = file("clipboard.tiff", "image/tiff");
    const dt = makeClipboardData([
      { kind: "file", type: "image/tiff", getAsFile: () => tiff },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([tiff]);
  });

  it("returns each of PNG, JPEG, GIF and WebP when present", () => {
    const png = file("a.png", "image/png");
    const jpg = file("b.jpg", "image/jpeg");
    const gif = file("c.gif", "image/gif");
    const webp = file("d.webp", "image/webp");
    const dt = makeClipboardData([
      { kind: "file", type: "image/png", getAsFile: () => png },
      { kind: "file", type: "image/jpeg", getAsFile: () => jpg },
      { kind: "file", type: "image/gif", getAsFile: () => gif },
      { kind: "file", type: "image/webp", getAsFile: () => webp },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([png, jpg, gif, webp]);
  });

  it("preserves clipboard order across multiple supported images", () => {
    const a = file("z.png", "image/png");
    const b = file("a.jpg", "image/jpeg");
    const c = file("m.gif", "image/gif");
    const dt = makeClipboardData([
      { kind: "file", type: "image/png", getAsFile: () => a },
      { kind: "file", type: "image/jpeg", getAsFile: () => b },
      { kind: "file", type: "image/gif", getAsFile: () => c },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([a, b, c]);
  });

  it("filters mixed supported, unsupported and string items to only supported files", () => {
    const png = file("a.png", "image/png");
    const jpg = file("b.jpg", "image/jpeg");
    const dt = makeClipboardData([
      { kind: "string", type: "text/plain", getAsFile: () => null },
      { kind: "file", type: "image/png", getAsFile: () => png },
      { kind: "file", type: "image/bmp", getAsFile: () => file("x.bmp", "image/bmp") },
      { kind: "string", type: "text/html", getAsFile: () => null },
      { kind: "file", type: "image/svg+xml", getAsFile: () => file("x.svg", "image/svg+xml") },
      { kind: "file", type: "image/jpeg", getAsFile: () => jpg },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([png, jpg]);
  });

  it("returns no files when a supported item whose getAsFile() is null is unavailable", () => {
    const png = file("a.png", "image/png");
    const dt = makeClipboardData([
      { kind: "file", type: "image/png", getAsFile: () => null },
      { kind: "file", type: "image/png", getAsFile: () => png },
      { kind: "file", type: "image/jpeg", getAsFile: () => null },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([]);
  });

  it("compares MIME types case-insensitively (IMAGE/PNG matches image/png)", () => {
    const png = file("a.png", "image/png");
    const jpg = file("b.jpg", "image/jpeg");
    const dt = makeClipboardData([
      { kind: "file", type: "IMAGE/PNG", getAsFile: () => png },
      { kind: "file", type: "Image/Jpeg", getAsFile: () => jpg },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([png, jpg]);
  });

  it("does not invoke getAsFile() for string or unsupported-MIME items", () => {
    const png = file("a.png", "image/png");
    const pngSpy = vi.fn(() => png);
    const stringSpy = vi.fn(() => null);
    const bmpSpy = vi.fn(() => file("x.bmp", "image/bmp"));
    const svgSpy = vi.fn(() => file("x.svg", "image/svg+xml"));
    const dt = makeClipboardData([
      { kind: "string", type: "text/plain", getAsFile: stringSpy },
      { kind: "file", type: "image/bmp", getAsFile: bmpSpy },
      { kind: "file", type: "image/svg+xml", getAsFile: svgSpy },
      { kind: "file", type: "image/png", getAsFile: pngSpy },
    ]);

    expect(extractImageFilesFromClipboard(dt)).toEqual([png]);
    expect(stringSpy).not.toHaveBeenCalled();
    expect(bmpSpy).not.toHaveBeenCalled();
    expect(svgSpy).not.toHaveBeenCalled();
    expect(pngSpy).toHaveBeenCalledTimes(1);
  });
});

describe("extractClipboardImageFiles", () => {
  it("distinguishes no supported image without reading text, HTML, or unsupported items", () => {
    const textSpy = vi.fn(() => null);
    const htmlSpy = vi.fn(() => null);
    const unsupportedSpy = vi.fn(() => file("a.bmp", "image/bmp"));
    const dt = makeClipboardData([
      { kind: "string", type: "text/plain", getAsFile: textSpy },
      { kind: "string", type: "text/html", getAsFile: htmlSpy },
      { kind: "file", type: "image/bmp", getAsFile: unsupportedSpy },
    ]);

    expect(extractClipboardImageFiles(dt)).toEqual({ kind: "none" });
    expect(textSpy).not.toHaveBeenCalled();
    expect(htmlSpy).not.toHaveBeenCalled();
    expect(unsupportedSpy).not.toHaveBeenCalled();
  });

  it("returns supported files in clipboard order with case-insensitive MIME matching", () => {
    const png = file("a.png", "image/png");
    const jpeg = file("b.jpg", "image/jpeg");
    const gif = file("c.gif", "image/gif");
    const webp = file("d.webp", "image/webp");
    const tiff = file("e.tiff", "image/tiff");
    const dt = makeClipboardData([
      { kind: "file", type: "IMAGE/PNG", getAsFile: () => png },
      { kind: "file", type: "Image/Jpeg", getAsFile: () => jpeg },
      { kind: "file", type: "image/GIF", getAsFile: () => gif },
      { kind: "file", type: "image/WebP", getAsFile: () => webp },
      { kind: "file", type: "IMAGE/TIFF", getAsFile: () => tiff },
    ]);

    expect(extractClipboardImageFiles(dt)).toEqual({
      kind: "files",
      files: [png, jpeg, gif, webp, tiff],
    });
  });

  it("reports a supported image that the browser cannot expose as a File", () => {
    const png = file("a.png", "image/png");
    const dt = makeClipboardData([
      { kind: "file", type: "IMAGE/TIFF", getAsFile: () => null },
      { kind: "file", type: "image/png", getAsFile: () => png },
    ]);

    expect(extractClipboardImageFiles(dt)).toEqual({
      kind: "unavailable",
      mime: "image/tiff",
    });
  });
});
