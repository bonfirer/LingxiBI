-- ============================================================
-- 027: GitHub OAuth login support.
-- Adds a github_id column to users so we can match GitHub accounts to local
-- accounts. The column is nullable (local-only users don't have one) and
-- unique (each GitHub account can only be linked to one local user).
-- ============================================================

ALTER TABLE users ADD COLUMN github_id BIGINT DEFAULT NULL;
ALTER TABLE users ADD UNIQUE INDEX uniq_github_id (github_id);
