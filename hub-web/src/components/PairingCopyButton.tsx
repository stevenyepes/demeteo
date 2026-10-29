import { Check, Copy } from 'lucide-react';
import { useState } from 'react';

import { Button } from '../../../src/components/ui/Button';

interface PairingCopyButtonProps {
  text: string;
  /** Accessible name; the visible label stays "Copy". */
  label: string;
}

/**
 * Absent where the page is not a secure context, which is also where passkeys
 * are not offered: the button is then not drawn rather than drawn dead.
 */
export function PairingCopyButton({ text, label }: PairingCopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const clipboard = typeof navigator === 'undefined' ? undefined : navigator.clipboard;
  if (clipboard === undefined) return null;

  function copy() {
    clipboard?.writeText(text).then(
      () => setCopied(true),
      () => setCopied(false),
    );
  }

  return (
    <Button size="sm" aria-label={label} onClick={copy} className="shrink-0">
      {copied ? <Check aria-hidden="true" className="h-3.5 w-3.5" /> : <Copy aria-hidden="true" className="h-3.5 w-3.5" />}
      {copied ? 'Copied' : 'Copy'}
    </Button>
  );
}
