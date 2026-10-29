-- An attachment is uploaded before the dispatch that names it, kept only
-- until it is fetched or its time runs out, and opened by no one but the
-- instance it was sent to (`docs/HUB.md` §6.3). Each statement here serves
-- one statement of `store/attachments.rs`.

-- A staged row has no request yet: it is bound once, at dispatch, to the
-- request that names it. The `(instance_id, request_id)` foreign key of 0001
-- is MATCH SIMPLE, so it goes unchecked while `request_id` is NULL and holds
-- again from the bind on.
ALTER TABLE attachments ALTER COLUMN request_id DROP NOT NULL;

-- `expires_at` is when the sweeper deletes the row: the upload time plus the
-- attachment TTL while staged, the request's own expiry once bound.
-- `sealed_digest` is the SHA-256 of the content, sealed under the row's data
-- key, so dispatch can sign it into the request without a plaintext digest
-- ever sitting beside the ciphertext.
--
-- The defaults exist only to fill rows that predate this migration, and are
-- dropped at once so that every later insert has to name both. Such a row
-- expires at the epoch and the first sweep removes it: it was never bound by
-- this lifecycle and nothing could fetch it.
ALTER TABLE attachments
  ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT 'epoch',
  ADD COLUMN sealed_digest BYTEA NOT NULL DEFAULT '\x';
ALTER TABLE attachments
  ALTER COLUMN expires_at DROP DEFAULT,
  ALTER COLUMN sealed_digest DROP DEFAULT;

-- The content length is derived from `size_bytes` by subtracting the nonce
-- and tag, which a negative length would turn into nonsense rather than an
-- error.
ALTER TABLE attachments
  ADD CONSTRAINT attachments_size_nonnegative CHECK (size_bytes >= 0);

-- The sweeper's delete, which runs once per tenant: one tenant's rows past
-- `expires_at` without walking the ones still waiting.
CREATE INDEX attachments_sweep ON attachments (user_id, expires_at);
