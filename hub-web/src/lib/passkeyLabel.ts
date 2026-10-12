const LABEL_MAX_CHARS = 64;
const CONTROL_CHAR = /\p{Cc}/u;

/**
 * Refuses what the Hub's `accepted_label` answers 400 for, so the page never offers to send it.
 * The bound is in characters, so code points are counted: `length` would count an emoji twice. A
 * control character is refused wherever it sits: Postgres stores no U+0000, and none belongs in a
 * name shown in a list.
 */
export function isUsablePasskeyLabel(label: string): boolean {
  const chars = Array.from(label).length;
  return chars >= 1 && chars <= LABEL_MAX_CHARS && !CONTROL_CHAR.test(label);
}
