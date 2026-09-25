import { AlertCircle } from 'lucide-react';

/** The inline amber note that says why a row above it will not fire. Two of
 *  these can stand at once — an unapplied edit and a still-running ancestor are
 *  independent — so neither may be folded into the other's text. */
export function GuardNote({ message }: { message: string }) {
  return (
    <div className="flex items-start gap-2 rounded-lg border border-amber-500/20 bg-amber-950/10 p-3 text-xs text-amber-300/90">
      <AlertCircle className="mt-px h-4 w-4 shrink-0 text-amber-400" />
      <span>{message}</span>
    </div>
  );
}
