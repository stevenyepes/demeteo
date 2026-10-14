-- The queue read: one instance's `queued` requests in `queue_seq` order, run
-- after every `hello` and on every wake-up. The `UNIQUE (user_id,
-- instance_id, id)` of 0001 finds the instance's rows but not in that order,
-- so without this the read fetches and sorts every request the instance ever
-- had, answered and expired ones included, since rows are never deleted.
-- Partial on `status`, so the index holds what is still waiting and nothing
-- else.
--
-- The predicate is the literal the read spells. Postgres uses a partial index
-- only for a statement it can prove implies the `WHERE` below, which a bound
-- `status = $3` under a generic plan is not.
CREATE INDEX instance_requests_queue_idx
  ON instance_requests (user_id, instance_id, queue_seq)
  WHERE status = 'queued';
