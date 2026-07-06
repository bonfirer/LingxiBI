-- ============================================================
-- 028: Report datasource runtime filters.
--
-- Lets a report dataset carry optional query conditions that are applied at
-- read time by wrapping the metric SQL as a subquery:
--     SELECT * FROM ( <metric_sql> ) AS _m WHERE ...
--
-- The column is NULL by default, so existing report datasources keep executing
-- their metric SQL verbatim (no filters => no wrapping, identical results).
-- ============================================================

ALTER TABLE report_datasources ADD COLUMN filters JSON DEFAULT NULL;
