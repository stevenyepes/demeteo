import { HubApiError } from '../lib/api';
import { WebAuthnCancelledError, WebAuthnUnavailableError } from '../lib/webauthn';

/**
 * A refusal from the Hub is shown as its code (`last_passkey`,
 * `self_endorsement`): the code is the stable name of the refusal, where the
 * message of a `HubApiError` only restates the path and the HTTP status.
 */
function messageOf(error: unknown): string {
  if (error instanceof WebAuthnUnavailableError) {
    return 'This browser does not offer passkeys here. Open the Hub over HTTPS in a browser that supports them.';
  }
  if (error instanceof WebAuthnCancelledError) {
    return 'The passkey prompt was cancelled, timed out, or the authenticator refused. Try again.';
  }
  if (error instanceof HubApiError) return error.code ?? error.message;
  return error instanceof Error ? error.message : 'Something went wrong.';
}

interface CeremonyErrorProps {
  error: unknown;
}

export function CeremonyError({ error }: CeremonyErrorProps) {
  return (
    <p
      role="alert"
      className="rounded-lg border border-ruby-500/30 bg-ruby-500/10 px-3 py-2 text-sm text-ruby-200 break-words"
    >
      {messageOf(error)}
    </p>
  );
}
