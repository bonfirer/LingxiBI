-- ============================================================
-- 032: Data pool parameters.
--
-- When the AI generates a parameterized query in a conversation (using
-- {{name}} / [[ ]] placeholders), the parameter definitions are stored on the
-- data pool so they carry over to any metric saved from that pool.
--
-- NULL/absent => the pool's SQL has no placeholders (unchanged behavior).
-- ============================================================

ALTER TABLE data_pools ADD COLUMN params JSON DEFAULT NULL;
