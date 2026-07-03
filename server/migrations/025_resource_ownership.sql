-- ============================================================
-- 025: Multi-user authorization — per-user ownership of personal
--      work products. "Shared infrastructure" (datasources, schema,
--      knowledge base, AI examples, LLM/SMTP/Feishu config) stays
--      admin-managed and readable by everyone, so it gets no owner column.
--
--      Each ALTER may fail with "duplicate column" on re-run; the migration
--      runner tolerates error 1060. The backfill UPDATEs are idempotent
--      (they only touch rows whose owner is still NULL).
-- ============================================================

ALTER TABLE reports                   ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE report_groups             ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE metric_pools              ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE metric_groups             ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE conversations             ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE alert_rules               ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE metric_snapshot_schedules ADD COLUMN owner_user_id INT DEFAULT NULL;
ALTER TABLE report_themes             ADD COLUMN owner_user_id INT DEFAULT NULL;

-- Backfill existing rows to the first (oldest) user, who is the initial admin.
-- Guarded so it is a no-op once owners are assigned.
UPDATE reports                   SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE report_groups             SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE metric_pools              SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE metric_groups             SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE conversations             SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE alert_rules               SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE metric_snapshot_schedules SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;
UPDATE report_themes             SET owner_user_id = (SELECT MIN(id) FROM users) WHERE owner_user_id IS NULL;

CREATE INDEX idx_reports_owner        ON reports(owner_user_id);
CREATE INDEX idx_report_groups_owner  ON report_groups(owner_user_id);
CREATE INDEX idx_metric_pools_owner   ON metric_pools(owner_user_id);
CREATE INDEX idx_metric_groups_owner  ON metric_groups(owner_user_id);
CREATE INDEX idx_conversations_owner  ON conversations(owner_user_id);
CREATE INDEX idx_alert_rules_owner    ON alert_rules(owner_user_id);
CREATE INDEX idx_snap_sched_owner     ON metric_snapshot_schedules(owner_user_id);
CREATE INDEX idx_report_themes_owner  ON report_themes(owner_user_id);
