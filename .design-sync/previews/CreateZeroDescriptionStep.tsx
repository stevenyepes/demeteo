import { CreateZeroDescriptionStep } from 'demeteo';

const noop = () => {};

/** A realistic feature description — this text becomes the Feature's
 *  `description` and is what the pipeline decomposes. */
export const Filled = () => (
  <div className="w-full max-w-xl">
    <CreateZeroDescriptionStep
      description={
        'Pool SSH connections per machine instead of dialling on every step.\n\n' +
        'Today each remote Step opens its own libssh2 session, which makes a ' +
        'ten-step run pay ten handshakes and occasionally trips the sshd rate ' +
        'limit. Keep one authenticated session per machine, hand out channels ' +
        'from it, and evict a session once it fails a keepalive.'
      }
      onChange={noop}
    />
  </div>
);

/** Empty — the placeholder states what the pipeline will do with the text,
 *  and the counter shows the 8-character floor. */
export const Empty = () => (
  <div className="w-full max-w-xl">
    <CreateZeroDescriptionStep description="" onChange={noop} />
  </div>
);

/** Just under the minimum, where the counter is the only feedback. */
export const TooShort = () => (
  <div className="w-full max-w-xl">
    <CreateZeroDescriptionStep description="fix ssh" onChange={noop} />
  </div>
);
