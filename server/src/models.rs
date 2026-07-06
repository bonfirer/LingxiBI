use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DataSource {
    pub id: i32,
    pub name: String,
    pub db_type: String,
    pub host: String,
    pub port: i32,
    pub database_name: String,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub status: String,
    /// Whether this is the single locked default datasource. At most one row is
    /// true at a time (enforced by the API).
    pub is_default: Option<bool>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDataSource {
    pub name: String,
    pub db_type: Option<String>,
    pub host: String,
    pub port: Option<i32>,
    pub database_name: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDataSource {
    pub name: Option<String>,
    pub host: Option<String>,
    pub port: Option<i32>,
    pub database_name: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub tables: Vec<TableInfo>,
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableInfo {
    pub name: String,
    pub comment: Option<String>,
    pub columns: Vec<ColumnInfo>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
    pub is_foreign_key: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Relationship {
    pub source_table: String,
    pub source_column: String,
    pub target_table: String,
    pub target_column: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KnowledgeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub columns: Vec<ColumnInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub r#type: String,
    pub on: String,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Conversation {
    pub id: i32,
    pub title: Option<String>,
    pub generation_status: Option<String>,
    pub generation_error: Option<String>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct Message {
    pub id: i32,
    pub conversation_id: i32,
    pub role: String,
    pub content: String,
    pub metadata: Option<serde_json::Value>,
    pub reasoning_content: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct DataPool {
    pub id: i32,
    pub conversation_id: Option<i32>,
    pub name: Option<String>,
    pub sql_query: String,
    pub datasource_id: i32,
    pub result_cache: Option<serde_json::Value>,
    pub row_count: Option<i32>,
    /// Parameter definitions when the pool's SQL uses `{{name}}` / `[[ ]]`
    /// placeholders (JSON array of `MetricParam`). Carried into a saved metric.
    pub params: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Report {
    pub id: i32,
    pub title: String,
    pub description: Option<String>,
    pub group_id: Option<i32>,
    pub pool_ids: serde_json::Value,
    pub config: serde_json::Value,
    pub data_cache: Option<serde_json::Value>,
    pub status: Option<String>,
    pub share_token: Option<String>,
    pub share_public: Option<bool>,
    pub layout_config: Option<serde_json::Value>,
    pub html_content: Option<String>,
    pub published_html: Option<String>,
    pub refresh_interval: Option<i32>,
    pub generation_status: Option<String>,
    pub generation_error: Option<String>,
    pub style_key: Option<String>,
    pub design_score: Option<serde_json::Value>,
    /// Report-level global filter controls, stored as a JSON array of
    /// `ReportFilter`. NULL/empty => no global filtering.
    pub report_filters: Option<serde_json::Value>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One report-level filter control. Its `value` is applied to every `target`
/// (dataset + column) whose dataset appears in the report. A blank value means
/// the control is inactive and contributes no condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportFilter {
    /// Stable key for the control (used by the UI).
    pub key: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Operator: =, !=, <>, >, >=, <, <=, LIKE, IN, BETWEEN.
    pub op: String,
    /// Current selected value. Scalar for most ops; array for IN / BETWEEN.
    #[serde(default)]
    pub value: serde_json::Value,
    #[serde(default)]
    pub targets: Vec<ReportFilterTarget>,
}

/// Where a report-level filter is applied: a dataset column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportFilterTarget {
    pub datasource_id: i32,
    pub column: String,
}

/// Request body to set a report's global filter controls.
#[derive(Debug, Deserialize)]
pub struct SetReportFilters {
    #[serde(default)]
    pub filters: Vec<ReportFilter>,
}

/// One entry in a report's SQL debug trace: the actual query executed for a
/// dataset, its bound parameters, and the result/timing. Only produced when the
/// debug panel is explicitly opened.
#[derive(Debug, Serialize)]
pub struct ReportDebugEntry {
    /// report_datasources.id
    pub dataset_id: i32,
    pub name: String,
    /// Underlying datasource id + engine.
    pub datasource_id: i32,
    pub db_type: String,
    /// The metric's stored SQL, before filters.
    pub base_sql: String,
    /// The SQL actually executed (base, or wrapped with a WHERE when filters
    /// are active).
    pub effective_sql: String,
    /// Bound parameter values, in order, for the effective SQL.
    pub params: Vec<serde_json::Value>,
    /// Number of active filter conditions (dataset + report-level combined).
    pub filter_count: usize,
    pub row_count: Option<usize>,
    pub duration_ms: u64,
    /// When this query was executed (UTC, RFC3339). This is the debug-time run,
    /// not a historical log entry.
    pub executed_at: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReport {
    pub title: String,
    pub description: Option<String>,
    pub pool_ids: Vec<i32>,
    pub group_id: Option<i32>,
    pub visualization_intent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LLMConfig {
    pub id: i32,
    pub provider: String,
    pub base_url: String,
    // The config endpoints return a masked key via `mask_config`. `skip_serializing`
    // is defense-in-depth so the raw key can never leak even if some future
    // endpoint returns this struct directly.
    #[serde(skip_serializing)]
    pub api_key: String,
    pub model: String,
    pub max_tokens: i32,
    pub temperature: f64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLLMConfig {
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<i32>,
    pub temperature: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryRequest {
    pub sql: String,
    pub datasource_id: i32,
    /// Optional parameter definitions (JSON array of `MetricParam`) — their
    /// defaults resolve `{{name}}` / `[[ ]]` placeholders when running the SQL.
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    /// Optional explicit parameter values (name -> value), overriding defaults.
    #[serde(default)]
    pub param_values: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Value>,
    pub row_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReportRenderConfig {
    pub visualizations: Vec<VisConfig>,
    pub layout: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisConfig {
    pub r#type: String,
    pub title: String,
    pub data_pool_id: i32,
    pub config: serde_json::Value,
}

/// Flexible render request — either manual config or AI prompt.
#[derive(Debug, Deserialize)]
pub struct RenderRequest {
    pub prompt: Option<String>,
    pub config: Option<ReportRenderConfig>,
    /// Optional saved theme to generate the dashboard in.
    pub theme_id: Option<i32>,
}

// ── Report Themes ──

/// A user-curated, reusable dashboard theme.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ReportTheme {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub style_prompt: Option<String>,
    /// Reference HTML template. Omitted from list responses to keep them light.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_html: Option<String>,
    pub emoji: String,
    pub source_report_id: Option<i32>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportThemeRequest {
    pub name: String,
    pub description: Option<String>,
    pub style_prompt: Option<String>,
    pub emoji: Option<String>,
    /// If set, capture this report's current HTML as the theme's sample template.
    pub source_report_id: Option<i32>,
    /// Explicit sample HTML (overrides source_report_id capture when provided).
    pub sample_html: Option<String>,
}

// ── AI Data Summary ──

/// Structured AI analysis of a report's data. Produced by the LLM as JSON and
/// cached in `report_summaries`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSummary {
    /// One-sentence key takeaway.
    pub headline: String,
    /// KPI-style findings backed by concrete numbers.
    #[serde(default)]
    pub highlights: Vec<String>,
    /// Trend observations (tied to snapshot deltas where available).
    #[serde(default)]
    pub trends: Vec<String>,
    /// Outliers, risks, or data-quality caveats.
    #[serde(default)]
    pub anomalies: Vec<String>,
    /// Actionable next steps.
    #[serde(default)]
    pub recommendations: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateSummaryRequest {
    /// UI language code ("zh" / "en"); controls the summary language.
    pub lang: Option<String>,
}

/// A single turn in the report Q&A conversation.
#[derive(Debug, Deserialize)]
pub struct ReportQaMessage {
    pub role: String,
    pub content: String,
}

/// Ask a grounded question about a report's data.
#[derive(Debug, Deserialize)]
pub struct ReportQaRequest {
    pub question: String,
    #[serde(default)]
    pub history: Vec<ReportQaMessage>,
    pub lang: Option<String>,
}

// ── Report Groups ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ReportGroup {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportGroup {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateReportGroup {
    pub name: Option<String>,
    pub description: Option<String>,
    pub sort_order: Option<i32>,
}

// ── Metric Groups ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MetricGroup {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMetricGroup {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMetricGroup {
    pub name: Option<String>,
    pub description: Option<String>,
    pub sort_order: Option<i32>,
}

// ── Metric Pools ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MetricPool {
    pub id: i32,
    pub name: String,
    pub description: Option<String>,
    pub sql_query: String,
    pub datasource_id: i32,
    pub group_id: Option<i32>,
    pub result_cache: Option<serde_json::Value>,
    pub row_count: Option<i32>,
    pub source_pool_id: Option<i32>,
    /// Declared parameters (JSON array of `MetricParam`). NULL/empty => the
    /// metric SQL has no placeholders and runs unchanged.
    pub params: Option<serde_json::Value>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A declared metric parameter. Its `name` matches the `{{name}}` placeholders
/// in the metric SQL; `default` (if set) is used when no runtime value is given.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricParam {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    /// UI hint: "text" | "number" | "date" | "daterange" | "enum". Optional.
    #[serde(default, rename = "type")]
    pub param_type: Option<String>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// Allowed values for enum-type parameters.
    #[serde(default)]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMetricPool {
    pub name: String,
    pub description: Option<String>,
    pub sql_query: String,
    pub datasource_id: i32,
    pub group_id: Option<i32>,
    pub source_pool_id: Option<i32>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMetricPool {
    pub name: Option<String>,
    pub description: Option<String>,
    pub sql_query: Option<String>,
    pub group_id: Option<i32>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// Request to move a report to a different group.
#[derive(Debug, Deserialize)]
pub struct MoveToGroup {
    pub group_id: Option<i32>,
}

/// AI auto-group suggestion request.
// ── Report Canvas ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ReportDataSource {
    pub id: i32,
    pub report_id: i32,
    pub metric_id: Option<i32>,
    pub name: String,
    pub sql_query: String,
    pub datasource_id: i32,
    pub result_cache: Option<serde_json::Value>,
    pub row_count: Option<i32>,
    /// Optional runtime query conditions, stored as a JSON array of
    /// `FilterCondition`. NULL/empty => the metric SQL runs unchanged.
    pub filters: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReportDataSource {
    pub metric_id: Option<i32>,
    pub name: String,
    pub sql_query: String,
    pub datasource_id: i32,
}

/// A single runtime query condition applied to a report dataset. Conditions are
/// applied by wrapping the metric SQL as a subquery and adding a `WHERE` clause;
/// values are always passed as bound parameters (never string-interpolated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterCondition {
    /// Column from the metric's output to filter on (validated as an identifier).
    pub column: String,
    /// Operator: =, !=, <>, >, >=, <, <=, LIKE, IN, BETWEEN.
    pub op: String,
    /// Bound value. Scalar for most ops; array for IN and [min, max] for BETWEEN.
    pub value: serde_json::Value,
}

/// Request body to set the runtime filters on a report dataset.
#[derive(Debug, Deserialize)]
pub struct SetReportDataSourceFilters {
    #[serde(default)]
    pub filters: Vec<FilterCondition>,
}

/// Canvas layout item — position and size in a 12-column grid.
/// Full canvas layout config stored in reports.layout_config
#[derive(Debug, Deserialize)]
pub struct PublishReport {
    pub status: String, // "draft" or "published"
}

#[derive(Debug, Deserialize)]
pub struct ShareReport {
    pub public: bool,
}

#[derive(Debug, Serialize)]
pub struct ShareInfo {
    pub share_token: String,
    pub public: bool,
    pub url: String,
}

// ── AI Knowledge Base ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct KnowledgeEntry {
    pub id: i32,
    pub datasource_id: i32,
    pub category: String,
    pub title: String,
    pub content: String,
    pub source: Option<String>,
    pub confidence: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateKnowledgeEntry {
    pub datasource_id: i32,
    pub category: Option<String>,
    pub title: String,
    pub content: String,
    pub source: Option<String>,
    pub confidence: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateKnowledgeEntry {
    pub category: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub confidence: Option<String>,
}

// ── AI Few-shot Examples ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AiExample {
    pub id: i32,
    pub datasource_id: i32,
    pub question: String,
    pub answer: String,
    pub category: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAiExample {
    pub datasource_id: i32,
    pub question: String,
    pub answer: String,
    pub category: Option<String>,
}

// ── Metric Snapshots ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MetricSnapshotSchedule {
    pub id: i32,
    pub metric_pool_id: i32,
    pub schedule_type: String,
    pub cron_expr: Option<String>,
    pub enabled: bool,
    pub retention_days: Option<i32>,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotSchedule {
    pub metric_pool_id: i32,
    pub schedule_type: String,
    pub cron_expr: Option<String>,
    pub retention_days: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSnapshotSchedule {
    pub schedule_type: Option<String>,
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,
    pub retention_days: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct MetricSnapshot {
    pub id: i32,
    pub metric_pool_id: i32,
    pub schedule_id: Option<i32>,
    pub snapshot_at: chrono::DateTime<chrono::Utc>,
    pub period_type: String,
    pub period_key: String,
    pub result_data: serde_json::Value,
    pub row_count: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Response for snapshot comparison (YoY / MoM / arbitrary)
#[derive(Debug, Serialize)]
pub struct SnapshotComparison {
    pub current: Option<MetricSnapshot>,
    pub previous: Option<MetricSnapshot>,
    pub period_type: String,
    pub current_key: String,
    pub previous_key: String,
}

// ── Table Descriptions ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct TableDescription {
    pub id: i32,
    pub datasource_id: i32,
    pub table_name: String,
    pub description: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertTableDescription {
    pub table_name: String,
    pub description: String,
}

// ── Column Descriptions ──

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct ColumnDescription {
    pub id: i32,
    pub datasource_id: i32,
    pub table_name: String,
    pub column_name: String,
    pub description: String,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpsertColumnDescription {
    pub table_name: String,
    pub column_name: String,
    pub description: String,
}

// ── Email Alerts ──

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SmtpConfig {
    pub id: i32,
    pub host: String,
    pub port: i32,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub from_email: String,
    pub from_name: String,
    pub use_tls: bool,
    pub enabled: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSmtpConfig {
    pub host: Option<String>,
    pub port: Option<i32>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from_email: Option<String>,
    pub from_name: Option<String>,
    pub use_tls: Option<bool>,
    pub enabled: Option<bool>,
}

/// Global Feishu (Lark) custom-bot configuration (single row, id = 1).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct FeishuConfig {
    pub id: i32,
    pub webhook_url: String,
    #[serde(skip_serializing)]
    pub secret: String,
    pub enabled: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFeishuConfig {
    pub webhook_url: Option<String>,
    pub secret: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AlertRule {
    pub id: i32,
    pub name: String,
    pub metric_pool_id: i32,
    pub condition_column: Option<String>,
    pub operator: String,
    pub threshold: f64,
    pub recipients: serde_json::Value,
    pub schedule_type: String,
    pub cron_expr: Option<String>,
    pub enabled: bool,
    pub subject_template: String,
    pub body_template: Option<String>,
    pub include_excel: bool,
    pub notify_feishu: bool,
    pub cooldown_minutes: i32,
    pub last_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_triggered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub owner_user_id: Option<i32>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAlertRule {
    pub name: String,
    pub metric_pool_id: i32,
    pub condition_column: Option<String>,
    pub operator: String,
    pub threshold: f64,
    pub recipients: Vec<String>,
    pub schedule_type: String,
    pub cron_expr: Option<String>,
    pub subject_template: Option<String>,
    pub body_template: Option<String>,
    pub include_excel: Option<bool>,
    pub notify_feishu: Option<bool>,
    pub cooldown_minutes: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlertRule {
    pub name: Option<String>,
    pub condition_column: Option<String>,
    pub operator: Option<String>,
    pub threshold: Option<f64>,
    pub recipients: Option<Vec<String>>,
    pub schedule_type: Option<String>,
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,
    pub subject_template: Option<String>,
    pub body_template: Option<String>,
    pub include_excel: Option<bool>,
    pub notify_feishu: Option<bool>,
    pub cooldown_minutes: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct AlertLog {
    pub id: i32,
    pub alert_rule_id: i32,
    pub evaluated_value: Option<f64>,
    pub triggered: bool,
    pub status: String,
    pub message: Option<String>,
    pub error: Option<String>,
    pub recipients: Option<serde_json::Value>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Request body for AI alert-template generation.
#[derive(Debug, Deserialize)]
pub struct GenerateAlertTemplateRequest {
    pub metric_pool_id: i32,
    pub operator: String,
    pub threshold: f64,
    pub condition_column: Option<String>,
    /// Optional free-form guidance from the user (tone, language, extra context).
    pub instructions: Option<String>,
    /// UI language code ("zh" / "en"); controls the generated email language.
    pub lang: Option<String>,
}

/// Request body for sending a test email for an alert rule.
#[derive(Debug, Deserialize)]
pub struct TestAlertEmail {
    pub recipients: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AlertTemplate {
    pub subject_template: String,
    pub body_template: String,
}
