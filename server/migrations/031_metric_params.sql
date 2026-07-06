-- ============================================================
-- 031: Metric parameters (parameterized metric SQL).
--
-- A metric can declare named parameters and use placeholders in its SQL:
--   {{name}}              -> required, replaced by a bound value/default
--   [[ ... {{name}} ... ]] -> optional block, included only when `name` has a
--                             value (otherwise the whole block is dropped)
--
-- `params` is a JSON array of parameter definitions (name/label/type/default).
-- NULL/absent => the metric has no parameters and runs exactly as before.
-- ============================================================

ALTER TABLE metric_pools ADD COLUMN params JSON DEFAULT NULL;
