-- Per-turn dollar budget (`--max-budget-usd`) as a peer of loop_iterations.
-- Nullable with no default: NULL = "inherit", so every pre-existing row
-- falls through the resolution chain (feature override -> project default
-- -> ExecutionDriver::DEFAULT_MAX_BUDGET_USD) for free. Stored as REAL so
-- sub-dollar role fractions round-trip exactly.
ALTER TABLE features ADD COLUMN max_budget_usd REAL;
ALTER TABLE project_settings ADD COLUMN default_max_budget_usd REAL;
