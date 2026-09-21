-- ============================================================
-- 034: Session invalidation support.
--
-- JWTs are self-contained, so before this column there was no way to revoke one:
-- deleting a user, resetting their password, or demoting them from admin left
-- their existing token working (with its original role) until it expired, up to
-- seven days later.
--
-- Session tokens now carry the issuing `token_version`, and every authenticated
-- request compares it against the stored value. Bumping this column immediately
-- invalidates every token previously issued to that user.
-- ============================================================

ALTER TABLE users ADD COLUMN token_version INT NOT NULL DEFAULT 0;
