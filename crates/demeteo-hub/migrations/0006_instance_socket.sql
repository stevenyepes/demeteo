-- `event_cursor` and `deliveries` carry a DEFAULT where 0003 and 0004 refuse
-- one. Theirs would have been a decision an insert could forget to state; 0 is
-- a fact about every row that exists before this file runs: no event stored,
-- never sent.
--
-- An instance is present when `connection_id IS NOT NULL`. No column says so
-- a second time.
ALTER TABLE instances
  ADD COLUMN install_id     TEXT,
  ADD COLUMN connection_id  UUID,
  ADD COLUMN connected_at   TIMESTAMPTZ,
  ADD COLUMN event_cursor   BIGINT NOT NULL DEFAULT 0 CHECK (event_cursor >= 0);

ALTER TABLE instance_requests
  ADD COLUMN deliveries      INTEGER NOT NULL DEFAULT 0 CHECK (deliveries >= 0),
  ADD COLUMN last_sent_at    TIMESTAMPTZ,
  ADD COLUMN outcome         TEXT CHECK (outcome IN ('accepted', 'refused')),
  ADD COLUMN refusal_reason  TEXT,
  ADD COLUMN answered_at     TIMESTAMPTZ,
  ADD CONSTRAINT instance_requests_status_check
    CHECK (status IN ('queued', 'expired', 'answered')),
  ADD CONSTRAINT instance_requests_answered_check
    CHECK ((status = 'answered') = (outcome IS NOT NULL)),
  ADD CONSTRAINT instance_requests_reason_check
    CHECK ((refusal_reason IS NOT NULL) = (outcome IS NOT DISTINCT FROM 'refused'));
