-- ============================================================
-- 030: Default datasource.
--
-- One datasource can be marked as the default (locked) datasource. At most one
-- row has is_default = 1 at a time; the API enforces this by clearing the flag
-- on the others when a new default is set.
-- ============================================================

ALTER TABLE datasources ADD COLUMN is_default TINYINT(1) DEFAULT 0;
