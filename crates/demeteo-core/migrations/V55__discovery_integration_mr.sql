-- The MR a Discovery's decomposition was published as, once one exists.
--
-- Unlike `base_branch` (V54), NULL here is a genuine third state — "no MR
-- opened yet" — not "use a default": a Discovery has no default MR to fall
-- back to, so a row with nothing published must say so rather than stand in
-- for one. `integration_mr_state` is NULL exactly when `integration_mr_url`
-- is, and is never read on its own.
ALTER TABLE discoveries ADD COLUMN integration_mr_url TEXT;
ALTER TABLE discoveries ADD COLUMN integration_mr_state TEXT;
