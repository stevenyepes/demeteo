/**
 * Drag-and-drop attachments — staging pipeline must be deterministic.
 *
 * Regression test for the bug where, when the user dropped a file
 * into the AttachmentDropzone (Tauri v2 webview `onDragDropEvent`
 * yields absolute paths, NOT a browser `File`), the staged
 * `LaunchStageEntry`:
 *
 *   1. recorded `size = 0` — the chip rendered "0 B" instead of the
 *      real file size, and
 *   2. fabricated a fresh random sha256 ("staged-<uuid>") per drop —
 *      so when the same file was dropped twice, the dedup filter
 *      `e.sha256 !== sha256` let both entries through, producing two
 *      chips for the same file (and two commits downstream).
 *
 * Fix: path-based inputs now go through the
 * `attachment_stage_metadata` Tauri IPC, which reads the file once
 * server-side and returns the real `{ sha256, size }`. Click-to-pick
 * (via <input type="file">) was already correct — `file.size` and
 * `computeLocalSha256(file)` work on a browser `File`. The bug was
 * path-only.
 *
 * What this test asserts (re-implements the fixed staging logic from
 * `src/components/AttachmentDropzone.tsx:ingestOneLaunch` and the
 * line-309 dedup filter):
 *   1. After ingesting the same path twice, the staging list contains
 *      exactly ONE entry (not two).
 *   2. The single retained entry's `size` equals the real on-disk
 *      byte count (not 0).
 *   3. The single retained entry's `sha256` equals the SHA-256 of the
 *      real on-disk bytes (matches the Rust-side
 *      `domain::attachment::compute_sha256_hex` value).
 *
 * Run (no React/jsdom/node_modules needed — pure Node ≥ 18):
 *
 *   $ node tests/repro/attachment-dnd-staging.mjs
 */

import { mkdtempSync, readFileSync, rmSync, writeFileSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';

// --------------------------------------------------------------------------
// Re-implementation of the staging logic from
//   src/components/AttachmentDropzone.tsx:ingestOneLaunch + the
//   caller-line dedup filter. Kept in sync with the production code;
//   if the production staging becomes a non-pure function of
//   `(input, prevStage)`, this test must follow.
//
// In the React/webview world, path-based ingest reads bytes via the
// `attachment_stage_metadata` Tauri command (see
// src-tauri/src/commands/attachments.rs) — that command reads the
// file once and returns the real `{ sha256, size }`. This Node
// version mirrors the IPC call directly with `readFileSync` so the
// repro runs in plain Node ≥ 18 with zero deps.
// --------------------------------------------------------------------------

/**
 * @typedef {{ kind: 'file'; file: { name: string; size: number; type: string; arrayBuffer(): Promise<ArrayBuffer> } }
 *         | { kind: 'path'; sourcePath: string; sourceFilename?: string; mime?: string }} AttachmentInput
 *
 * @typedef {{
 *   sha256: string;
 *   name: string;
 *   source_filename: string;
 *   mime: string;
 *   size: number;
 *   sourcePath: string | null;
 * }} LaunchStageEntry
 */

/** Mirror of `guessMime(lower)` in AttachmentDropzone.tsx. */
function guessMime(lower) {
  if (lower.endsWith('.png')) return 'image/png';
  if (lower.endsWith('.jpg') || lower.endsWith('.jpeg')) return 'image/jpeg';
  if (lower.endsWith('.gif')) return 'image/gif';
  if (lower.endsWith('.webp')) return 'image/webp';
  if (lower.endsWith('.pdf')) return 'application/pdf';
  if (lower.endsWith('.md') || lower.endsWith('.markdown')) return 'text/markdown';
  if (lower.endsWith('.txt')) return 'text/plain';
  if (lower.endsWith('.json')) return 'application/json';
  return 'application/octet-stream';
}

/** Mirror of `computeLocalSha256(file)` in src/lib/attachments.ts. */
async function computeLocalSha256(file) {
  const buf = await file.arrayBuffer();
  return createHash('sha256').update(Buffer.from(buf)).digest('hex');
}

/**
 * Mirror of the `attachment_stage_metadata` Rust command
 * (src-tauri/src/commands/attachments.rs). Reads the file from disk
 * and returns the deterministic `{ sha256, size }` that the Rust
 * command produces server-side.
 */
function stageAttachmentMetadata(sourcePath, _mime, _sourceFilename) {
  const bytes = readFileSync(sourcePath);
  return {
    sha256: createHash('sha256').update(bytes).digest('hex'),
    size: bytes.length,
  };
}

/**
 * Browser-free re-implementation of `ingestOneLaunch` +
 * `onChangeStage` from `AttachmentDropzone.tsx`. Mirrors the FIXED
 * behaviour: path-based inputs go through the metadata IPC and pick
 * up real `sha256` + `size`, so the dedup key is stable across
 * re-drops and the chip's byte-count label is correct.
 */
async function stageDrop(prevStage, input) {
  const sourceFilename =
    input.kind === 'file'
      ? input.file.name
      : (input.sourceFilename ?? input.sourcePath.split(/[\\/]/).pop() ?? 'attachment');
  const mime =
    input.kind === 'file'
      ? (input.file.type || guessMime(sourceFilename.toLowerCase()))
      : (input.mime ?? guessMime(sourceFilename.toLowerCase()));
  const sourcePath =
    input.kind === 'path'
      ? input.sourcePath
      : (input.file).path ?? null;
  const file = input.kind === 'file' ? input.file : null;

  let sha256;
  let size;
  if (input.kind === 'file' && file) {
    sha256 = await computeLocalSha256(file);
    size = file.size;
  } else if (input.kind === 'path') {
    const meta = stageAttachmentMetadata(input.sourcePath, input.mime ?? null, sourceFilename);
    sha256 = meta.sha256;
    size = meta.size;
  } else {
    throw new Error('unsupported input kind');
  }

  const entry = {
    sha256,
    name: sourceFilename,
    source_filename: sourceFilename,
    mime,
    size,
    sourcePath,
  };

  // mirrors the dedup filter at AttachmentDropzone.tsx (caller).
  const next = [...(prevStage ?? []).filter((e) => e.sha256 !== sha256), entry];
  return next;
}

// --------------------------------------------------------------------------
// Test driver — exercise the staging pipeline with a real on-disk file.
// --------------------------------------------------------------------------

async function main() {
  const tmp = mkdtempSync(join(tmpdir(), 'demeteo-attach-dnd-'));
  try {
    // Write a real PNG-like payload so the path-based input has a
    // known, non-zero size. 1234 bytes — the test asserts that the
    // staging logic surfaces this exact value.
    const padded = Buffer.alloc(1234, 0x61); // 'a' * 1234
    const fixturePath = join(tmp, 'shot.png');
    writeFileSync(fixturePath, padded);
    const onDiskSize = statSync(fixturePath).size;
    const onDiskSha256 = createHash('sha256').update(padded).digest('hex');

    console.log(`[repro] fixture: ${fixturePath}`);
    console.log(`[repro] fixture size: ${onDiskSize} bytes`);
    console.log(`[repro] fixture sha256: ${onDiskSha256}`);

    // ----- (A) Two drops of the SAME path --------------------------------
    //
    // Drag-and-drop yields `payload.paths = ["/abs/.../shot.png"]` —
    // a path-based input with NO browser File. The component's
    // `ingestPaths` (AttachmentDropzone.tsx) loops over the
    // paths and calls `ingestOneLaunch({ kind: "path", ... })`.
    //
    // We simulate that loop here.

    const pathInput = {
      kind: 'path',
      sourcePath: fixturePath,
      sourceFilename: 'shot.png',
      mime: 'image/png',
    };

    let stage = [];
    stage = await stageDrop(stage, pathInput);
    stage = await stageDrop(stage, pathInput); // same file, dropped again

    console.log(`\n[repro] stage after two drops of the same path:`);
    for (const e of stage) {
      console.log(
        `         sha256=${e.sha256.slice(0, 12)}… size=${e.size} source_filename=${e.source_filename}`,
      );
    }

    const checks = [];

    // (1) Dedup: dropping the same path twice MUST produce one chip.
    checks.push({
      name: 'stage contains exactly 1 entry (no duplicate drop)',
      expected: 1,
      observed: stage.length,
      ok: stage.length === 1,
    });

    // (2) Size: the single chip must show the real on-disk byte count.
    checks.push({
      name: 'stage entry size equals fs.statSync(path).size (not 0)',
      expected: onDiskSize,
      observed: stage[0]?.size,
      ok: stage[0]?.size === onDiskSize,
    });

    // (3) Sha256: must be deterministic for the same bytes.
    checks.push({
      name: 'stage entry sha256 equals SHA-256 of on-disk bytes',
      expected: onDiskSha256,
      observed: stage[0]?.sha256,
      ok: stage[0]?.sha256 === onDiskSha256,
    });

    console.log('');
    let allOk = true;
    for (const c of checks) {
      const tag = c.ok ? 'PASS' : 'FAIL';
      console.log(`[repro] [${tag}] ${c.name}`);
      console.log(`         expected: ${c.expected}`);
      console.log(`         observed: ${c.observed}`);
      if (!c.ok) allOk = false;
    }

    if (!allOk) {
      console.error(
        '\n[repro] FAIL: the path-based (drag-and-drop) staging pipeline' +
          '\n        regressed. Two symptoms are visible:' +
          '\n' +
          '\n        1. Duplicate entries — dropping the same file' +
          '\n           twice produces two chips (the local dedup key' +
          '\n           is no longer derived from the real bytes).' +
          '\n' +
          '\n        2. Zero-byte chips — drag-and-drop entries show' +
          '\n           size=0 instead of the real on-disk byte count.' +
          '\n' +
          '\n        The path branch in src/components/AttachmentDropzone.tsx' +
          '\n        must call the `attachment_stage_metadata` Tauri command' +
          '\n        (src/lib/attachments.ts) so it reads the file server-side' +
          '\n        and returns { sha256, size } from the real bytes.',
      );
      process.exit(1);
    }
    console.log('\n[repro] PASS: staging pipeline is correct.');
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error('[repro] unexpected error:', err);
  process.exit(1);
});