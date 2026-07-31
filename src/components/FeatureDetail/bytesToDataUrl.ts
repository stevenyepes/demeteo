/**
 * Encode `bytes` as a `data:<mime>;base64,…` URL for use as the `src`
 * of an inline `<img>` in the attachment preview Modal.
 *
 * Exported (named) so the conversion is unit-testable in isolation.
 * The chunked `fromCharCode` walk avoids blowing the JS argument limit
 * on the larger image cap (10 MiB).
 */
export function bytesToDataUrl(mime: string, bytes: Uint8Array): string {
  let binary = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const slice = bytes.subarray(i, Math.min(i + CHUNK, bytes.length));
    binary += String.fromCharCode.apply(null, Array.from(slice));
  }
  return `data:${mime};base64,${btoa(binary)}`;
}
