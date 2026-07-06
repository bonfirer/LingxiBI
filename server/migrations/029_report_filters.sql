-- ============================================================
-- 029: Report-level global filters.
--
-- A report can define named filter controls (e.g. "date range", "region")
-- that map to one or more dataset columns. At read time each control's value
-- is translated into per-dataset conditions and combined (AND) with any
-- dataset-level filters.
--
-- NULL by default: reports with no global filters behave exactly as before.
-- ============================================================

ALTER TABLE reports ADD COLUMN report_filters JSON DEFAULT NULL;
