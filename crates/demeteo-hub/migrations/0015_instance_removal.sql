-- When an instance was removed from the Hub (`docs/HUB.md` §4 *Revocation*):
-- by its tenant from the browser, or by the instance itself as it unpairs.
-- NULL is an instance still in the fleet.
--
-- A column and not a delete. `run_mirror`, `snapshots`, `instance_requests`
-- and `attachments` all reference `instances` `ON DELETE CASCADE`, and the run
-- history of a removed instance is kept: the runs list still shows it, with
-- the instance named as removed. The row's device keys are revoked in the
-- same transaction, by `device_keys.revoked_at`, which 0001 added and nothing
-- wrote until now.
ALTER TABLE instances
  ADD COLUMN removed_at TIMESTAMPTZ;
