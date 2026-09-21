-- ============================================================
-- 021: Email Alerts - metric-based email alerting with AI templates,
--      scheduled evaluation, and Excel data attachments.
-- ============================================================

-- Global SMTP configuration (single row, id = 1)
CREATE TABLE IF NOT EXISTS smtp_config (
    id          INT PRIMARY KEY DEFAULT 1,
    host        VARCHAR(255) NOT NULL DEFAULT '',
    port        INT NOT NULL DEFAULT 465,
    username    VARCHAR(255) NOT NULL DEFAULT '',
    password    VARCHAR(255) NOT NULL DEFAULT '',
    from_email  VARCHAR(255) NOT NULL DEFAULT '',
    from_name   VARCHAR(255) NOT NULL DEFAULT 'HISENSE LingxiBI',
    use_tls     TINYINT(1) NOT NULL DEFAULT 1,
    enabled     TINYINT(1) NOT NULL DEFAULT 0,
    created_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Seed the single config row
INSERT INTO smtp_config (id, host, port, from_name) VALUES (1, '', 465, 'HISENSE LingxiBI')
ON DUPLICATE KEY UPDATE id = id;

-- Alert rules: evaluate a metric on a schedule and email when a condition holds
CREATE TABLE IF NOT EXISTS alert_rules (
    id                INT AUTO_INCREMENT PRIMARY KEY,
    name              VARCHAR(255) NOT NULL,
    metric_pool_id    INT NOT NULL,
    -- Which column in the metric result to evaluate. NULL = first numeric column.
    condition_column  VARCHAR(255) DEFAULT NULL,
    -- Comparison operator: gt, gte, lt, lte, eq, ne
    operator          VARCHAR(10) NOT NULL DEFAULT 'gt',
    threshold         DOUBLE NOT NULL DEFAULT 0,
    -- JSON array of recipient email addresses
    recipients        JSON NOT NULL,
    -- Scheduling (same semantics as metric_snapshot_schedules)
    schedule_type     VARCHAR(20) NOT NULL DEFAULT 'daily',
    cron_expr         VARCHAR(100) DEFAULT NULL,
    enabled           TINYINT(1) NOT NULL DEFAULT 1,
    -- Email template (AI-generatable). Body supports {{placeholders}}.
    subject_template  VARCHAR(500) NOT NULL DEFAULT '',
    body_template     MEDIUMTEXT,
    include_excel     TINYINT(1) NOT NULL DEFAULT 1,
    -- Minimum minutes between two triggered alerts (anti-spam). 0 = no cooldown.
    cooldown_minutes  INT NOT NULL DEFAULT 0,
    last_run_at       TIMESTAMP NULL,
    next_run_at       TIMESTAMP NULL,
    last_triggered_at TIMESTAMP NULL,
    created_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at        TIMESTAMP DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    FOREIGN KEY (metric_pool_id) REFERENCES metric_pools(id) ON DELETE CASCADE,
    INDEX idx_alert_enabled (enabled, next_run_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

-- Alert evaluation / send history
CREATE TABLE IF NOT EXISTS alert_logs (
    id              INT AUTO_INCREMENT PRIMARY KEY,
    alert_rule_id   INT NOT NULL,
    evaluated_value DOUBLE DEFAULT NULL,
    triggered       TINYINT(1) NOT NULL DEFAULT 0,
    -- sent | failed | not_triggered | skipped
    status          VARCHAR(20) NOT NULL,
    message         TEXT,
    error           TEXT,
    recipients      JSON,
    created_at      TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (alert_rule_id) REFERENCES alert_rules(id) ON DELETE CASCADE,
    INDEX idx_alert_log_rule (alert_rule_id, created_at DESC)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
