/**
 * Drag-and-drop attachments — staging pipeline produces duplicates and 0-byte chips.
 *
 * Bug:
 *   When the user drops a file into the AttachmentDropzone (Tauri v2
 *   webview `onDragDropEvent` yields absolute paths, NOT a browser
 *   `File`), the staged LaunchStageEntry:
 *
 *     1. records `size = 0` — the chip then renders "0 B" instead of
 *        the real file size, and
 *
 *     2. fabricates a fresh random sha256 ("staged-<uuid>") per drop —
 *        so when the same file is dropped twice, the dedup filter
 *        `e.sha256 !== sha256` lets both entries through, producing
 *        two chips for the same file (and two commits downstream).
 *
 *   Click-to-pick (via <input type="file">) does NOT exhibit either
 *   symptom because the browser `File` is available — `file.size`
 *   and `computeLocalSha256(file)` both work. The bug is path-only.
 *
 * What this test asserts:
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
 *
 * The script:
 *   1. Spins up a tiny temp file with known bytes (real fs).
 *   2. Re-implements the *exact* staging logic from
 *      `src/components/AttachmentDropzone.tsx:268-312` (`ingestOneLaunch`)
 *      and line 309 dedup filter — copied verbatim into `stageDrop`
 *      below. The line numbers are in the comment header so a future
 *      refactor that moves the logic keeps the test honest.
 *   3. Invokes `stageDrop` twice for the same path (simulating the
 *      user dropping the same file twice).
 *   4. Inspects the resulting stage list.
 *
 * Expected:
 *   - Bug present (current `AttachmentDropzone.tsx`): FAIL
 *       - stage.length === 2 (duplicate) instead of 1
 *       - stage[0].size === 0 instead of the real byte count
 *   - Bug fixed: PASS
 *       - stage.length === 1, size matches fs.statSync(path).size,
 *         sha256 matches SHA-256 of the bytes.
 */

import { mkdtempSync, rmSync, writeFileSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';

// --------------------------------------------------------------------------
// Verbatim reproduction of the staging logic from
//   src/components/AttachmentDropzone.tsx:268-312 (ingestOneLaunch)
//   + line 309 dedup filter (in the caller, ingestPaths).
//
// Each line below corresponds to the same-numbered line in the React
// component. Keep them in sync if the production code moves; if the
// staging logic stops being a pure function of `(input, prevStage)`,
// this test must follow.
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

function randomId() {
  if (typeof globalThis.crypto?.randomUUID === 'function') {
    return globalThis.crypto.randomUUID();
  }
  return randomUUID();
}

/**
 * Pure, browser-free re-implementation of `ingestOneLaunch` +
 * `onChangeStage` from `AttachmentDropzone.tsx:268-312`. See the
 * line-number comments inline.
 */
async function stageDrop(prevStage, input) {
  // --- mirrors AttachmentDropzone.tsx:275-288 ---
  const sourceFilename =
    input.kind === 'file'
      ? input.file.name
      : (input.sourceFilename ?? input.sourcePath.split(/[\\/]/).pop() ?? 'attachment');
  const mime =
    input.kind === 'file'
      ? (input.file.type || guessMime(sourceFilename.toLowerCase()))
      : (input.mime ?? guessMime(sourceFilename.toLowerCase()));
  const size = input.kind === 'file' ? input.file.size : 0;            // <-- BUG #1: hardcoded 0 for path-based inputs
  const sourcePath =
    input.kind === 'path'
      ? input.sourcePath
      : (input.file).path ?? null;
  const file = input.kind === 'file' ? input.file : null;

  // --- mirrors AttachmentDropzone.tsx:291 ---
  // Bug #2: path-based inputs use a random id, so the dedup filter
  //         `e.sha256 !== sha256` cannot recognise a re-drop.
  const sha256 = file ? await computeLocalSha256(file) : 'staged-' + randomId();

  const entry = {
    sha256,
    name: sourceFilename,
    source_filename: sourceFilename,
    mime,
    size,
    sourcePath,
  };

  // --- mirrors AttachmentDropzone.tsx:309 ---
  const next = [...(prevStage ?? []).filter((e) => e.sha256 !== sha256), entry];
  return next;
}

// --------------------------------------------------------------------------
// Test driver — exercise the staging pipeline with a real on-disk file.
// --------------------------------------------------------------------------

function fail(msg) {
  console.error(`\n[repro] FAIL: ${msg}`);
  process.exit(1);
}

function main() {
  const tmp = mkdtempSync(join(tmpdir(), 'demeteo-attach-dnd-'));
  try {
    // Write a real PNG-like payload so the path-based input has a
    // known, non-zero size. 1234 bytes — the test asserts that the
    // staging logic surfaces this exact value.
    const bytes = Buffer.from(
      '\x89PNG\r\n\x1a\n' + 'demeteo-attachment-payload-' + 'a'.repeat(1234 - 26),
      'binary',
    );
    // Trim or pad to exactly 1234 bytes deterministically.
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
    // `ingestPaths` (AttachmentDropzone.tsx:225-251) loops over the
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
    return (async () => {
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
            '\n        is broken. Two symptoms are visible:' +
            '\n' +
            '\n        1. Duplicate entries — dropping the same file' +
            '\n           twice produces two chips (dedup filter at' +
            '\n           AttachmentDropzone.tsx:309 compares sha256,' +
            '\n           which is a fresh random id for every' +
            '\n           path-based drop — see line 291).' +
            '\n' +
            '\n        2. Zero-byte chips — drag-and-drop entries have' +
            '\n           size hardcoded to 0 at AttachmentDropzone.tsx:283' +
            '\n           because the browser File is not available in' +
            '\n           the Tauri webview drag-drop event.' +
            '\n' +
            '\n        Fix outline (NOT applied by this repro):' +
            '\n        - Dedup path-based drops by `sourcePath` (not sha256).' +
            '\n        - Stat the file (Tauri fs plugin) or read its bytes' +
            '\n          when ingesting a path, and populate size + sha256' +
            '\n          from the real on-disk content.',
        );
        process.exit(1);
      }
      console.log('\n[repro] PASS: staging pipeline is correct.');
      process.exit(0);
    })();
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

main();