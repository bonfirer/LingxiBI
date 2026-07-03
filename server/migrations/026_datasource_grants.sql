-- ============================================================
-- 026: Datasource-level authorization (v2).
--
-- Datasources remain admin-managed. Members can only see/use a datasource if
-- they have been granted access. Admins implicitly access every datasource, so
-- they need no rows here.
--
-- No backfill: after this migration, existing datasources have no member grants
-- (admins still see everything). An admin grants access from the UI as needed.
-- ============================================================

CREATE TABLE IF NOT EXISTS datasource_grants (
    id             INT AUTO_INCREMENT PRIMARY KEY,
    datasource_id  INT NOT NULL,
    user_id        INT NOT NULL,
    created_at     TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uniq_ds_user (datasource_id, user_id),
    FOREIGN KEY (datasource_id) REFERENCES datasources(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
