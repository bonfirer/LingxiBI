// ── Shared types — single source of truth ──
// Import from here instead of duplicating across api.ts and store files.

// ── Data Sources ──
export interface DataSource {
  id: number;
  name: string;
  db_type: string;
  host: string;
  port: number;
  database_name: string;
  username: string;
  status: string;
  is_default?: boolean;
  created_at?: string;
  updated_at?: string;
}

export interface CreateDataSourcePayload {
  name: string;
  db_type?: string;
  host: string;
  port?: number;
  database_name: string;
  username: string;
  password: string;
}

export interface UpdateDataSourcePayload {
  name?: string;
  host?: string;
  port?: number;
  database_name?: string;
  username?: string;
  password?: string;
}

// ── Schema ──
export interface SchemaInfo {
  tables: TableInfo[];
  relationships: Relationship[];
}

export interface TableInfo {
  name: string;
  comment?: string | null;
  columns: ColumnInfo[];
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  is_primary_key: boolean;
  is_foreign_key: boolean;
  comment?: string | null;
}

export interface Relationship {
  source_table: string;
  source_column: string;
  target_table: string;
  target_column: string;
}

// ── Knowledge Graph ──
export interface GraphNode {
  id: string;
  label: string;
  columns: ColumnInfo[];
}

export interface GraphEdge {
  source: string;
  target: string;
  type: string;
  on: string;
}

export interface KnowledgeGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

// ── Conversations ──
export interface Conversation {
  id: number;
  title?: string;
  generation_status?: string;
  generation_error?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface Message {
  id: number;
  conversation_id: number;
  role: string;
  content: string;
  metadata?: unknown;
  created_at?: string;
}

// ── Data Pools ──
export interface DataPool {
  id: number;
  conversation_id?: number;
  name?: string;
  sql_query: string;
  datasource_id: number;
  result_cache?: unknown;
  row_count?: number;
  created_at?: string;
}

// ── Reports ──
export interface Report {
  id: number;
  title: string;
  description?: string;
  group_id?: number | null;
  pool_ids: number[];
  config: ReportConfig;
  data_cache?: unknown;
  status?: string;
  share_token?: string | null;
  share_public?: boolean;
  layout_config?: CanvasLayout | null;
  html_content?: string | null;
  published_html?: string | null;
  refresh_interval?: number | null;
  generation_status?: string | null;
  generation_error?: string | null;
  style_key?: string | null;
  design_score?: {
    layout: number;
    color: number;
    typography: number;
    responsiveness: number;
    data_viz: number;
    total: number;
  } | null;
  report_filters?: ReportFilter[] | null;
  created_at?: string;
  updated_at?: string;
}

export interface ReportFilterTarget {
  datasource_id: number;
  column: string;
}

export interface ReportDebugEntry {
  dataset_id: number;
  name: string;
  datasource_id: number;
  db_type: string;
  base_sql: string;
  effective_sql: string;
  params: unknown[];
  filter_count: number;
  row_count?: number | null;
  duration_ms: number;
  executed_at: string;
  error?: string | null;
}

export interface ReportFilter {
  key: string;
  label?: string;
  op: FilterCondition['op'];
  value: unknown;
  targets: ReportFilterTarget[];
}

export interface ReportConfig {
  visualizations: VisConfig[];
  layout: string;
}

export interface VisConfig {
  type: string;
  title: string;
  data_pool_id: number;
  config: Record<string, unknown>;
}

export interface CanvasLayout {
  items: LayoutItem[];
  row_height: number;
  cols: number;
}

export interface LayoutItem {
  i: string;
  x: number;
  y: number;
  w: number;
  h: number;
  chart_type: string;
  title: string;
  datasource_id: number;
  config: Record<string, unknown>;
}

export interface CreateReportPayload {
  title: string;
  description?: string;
  pool_ids: number[];
  group_id?: number | null;
  visualization_intent?: string;
}

export interface FilterCondition {
  column: string;
  op: '=' | '!=' | '>' | '>=' | '<' | '<=' | 'LIKE' | 'IN' | 'BETWEEN';
  value: unknown;
}

export interface ReportDataSource {
  id: number;
  report_id: number;
  metric_id?: number | null;
  name: string;
  sql_query: string;
  datasource_id: number;
  result_cache?: unknown;
  row_count?: number;
  filters?: FilterCondition[] | null;
  created_at?: string;
}

export interface ShareInfo {
  share_token: string;
  public: boolean;
  url: string;
}

// ── Report Groups ──
export interface ReportGroup {
  id: number;
  name: string;
  description?: string;
  sort_order: number;
  created_at?: string;
  updated_at?: string;
}

// ── Metric Groups ──
export interface MetricGroup {
  id: number;
  name: string;
  description?: string;
  sort_order: number;
  created_at?: string;
  updated_at?: string;
}

// ── Metric Pools ──
export interface MetricParam {
  name: string;
  label?: string;
  type?: 'text' | 'number' | 'date' | 'daterange' | 'enum';
  default?: unknown;
  options?: string[];
}

export interface MetricPool {
  id: number;
  name: string;
  description?: string;
  sql_query: string;
  datasource_id: number;
  group_id?: number | null;
  result_cache?: unknown;
  row_count?: number;
  source_pool_id?: number | null;
  params?: MetricParam[] | null;
  created_at?: string;
  updated_at?: string;
}

export interface CreateMetricPayload {
  name: string;
  description?: string;
  sql_query: string;
  datasource_id: number;
  group_id?: number | null;
  source_pool_id?: number | null;
  params?: MetricParam[] | null;
}

// ── LLM Config ──
export interface LLMConfig {
  id: number;
  provider: string;
  base_url: string;
  api_key: string;
  model: string;
  max_tokens: number;
  temperature: number;
}

export interface UpdateLLMConfigPayload {
  provider?: string;
  base_url?: string;
  api_key?: string;
  model?: string;
  max_tokens?: number;
  temperature?: number;
}

// ── Query ──
export interface QueryResult {
  columns: string[];
  rows: unknown[];
  row_count: number;
}

// ── Knowledge Base ──
export interface KnowledgeEntry {
  id: number;
  datasource_id: number;
  category: string;
  title: string;
  content: string;
  source?: string;
  confidence?: string;
  created_at?: string;
  updated_at?: string;
}

export interface CreateKnowledgeEntryPayload {
  datasource_id: number;
  category?: string;
  title: string;
  content: string;
  source?: string;
  confidence?: string;
}

// ── Metric Snapshots ──
export interface SnapshotSchedule {
  id: number;
  metric_pool_id: number;
  schedule_type: string;
  cron_expr?: string | null;
  enabled: boolean;
  retention_days?: number | null;
  last_run_at?: string | null;
  next_run_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface CreateSnapshotSchedulePayload {
  metric_pool_id: number;
  schedule_type: string;
  cron_expr?: string | null;
  retention_days?: number | null;
}

export interface UpdateSnapshotSchedulePayload {
  schedule_type?: string;
  cron_expr?: string | null;
  enabled?: boolean;
  retention_days?: number | null;
}

export interface MetricSnapshot {
  id: number;
  metric_pool_id: number;
  schedule_id: number;
  snapshot_at: string;
  period_type: string;
  period_key: string;
  result_data: unknown;
  row_count?: number;
  created_at?: string;
}

export interface SnapshotComparison {
  current?: MetricSnapshot | null;
  previous?: MetricSnapshot | null;
  period_type: string;
  current_key: string;
  previous_key: string;
}

// ── Email Alerts ──
export interface SmtpConfig {
  id: number;
  host: string;
  port: number;
  username: string;
  password_set: boolean;
  from_email: string;
  from_name: string;
  use_tls: boolean;
  enabled: boolean;
}

export interface UpdateSmtpConfigPayload {
  host?: string;
  port?: number;
  username?: string;
  password?: string;
  from_email?: string;
  from_name?: string;
  use_tls?: boolean;
  enabled?: boolean;
}

export interface FeishuConfig {
  id: number;
  webhook_url: string;
  secret_set: boolean;
  enabled: boolean;
}

export interface UpdateFeishuConfigPayload {
  webhook_url?: string;
  secret?: string;
  enabled?: boolean;
}

export type AlertOperator = 'gt' | 'gte' | 'lt' | 'lte' | 'eq' | 'ne';

export interface AlertRule {
  id: number;
  name: string;
  metric_pool_id: number;
  condition_column?: string | null;
  operator: AlertOperator;
  threshold: number;
  recipients: string[];
  schedule_type: string;
  cron_expr?: string | null;
  enabled: boolean;
  subject_template: string;
  body_template?: string | null;
  include_excel: boolean;
  notify_feishu: boolean;
  cooldown_minutes: number;
  last_run_at?: string | null;  next_run_at?: string | null;
  last_triggered_at?: string | null;
  created_at?: string;
  updated_at?: string;
}

export interface CreateAlertRulePayload {
  name: string;
  metric_pool_id: number;
  condition_column?: string | null;
  operator: AlertOperator;
  threshold: number;
  recipients: string[];
  schedule_type: string;
  cron_expr?: string | null;
  subject_template?: string;
  body_template?: string | null;
  include_excel?: boolean;
  notify_feishu?: boolean;
  cooldown_minutes?: number;
}

export interface UpdateAlertRulePayload {
  name?: string;
  condition_column?: string | null;
  operator?: AlertOperator;
  threshold?: number;
  recipients?: string[];
  schedule_type?: string;
  cron_expr?: string | null;
  enabled?: boolean;
  subject_template?: string;
  body_template?: string | null;
  include_excel?: boolean;
  notify_feishu?: boolean;
  cooldown_minutes?: number;
}

export interface AlertLog {
  id: number;
  alert_rule_id: number;
  evaluated_value?: number | null;
  triggered: boolean;
  status: string;
  message?: string | null;
  error?: string | null;
  recipients?: string[] | null;
  created_at?: string;
}

export interface AlertTemplate {
  subject_template: string;
  body_template: string;
}

export interface GenerateAlertTemplatePayload {
  metric_pool_id: number;
  operator: AlertOperator;
  threshold: number;
  condition_column?: string | null;
  instructions?: string;
  lang?: string;
}

// ── Feature Wishes ──

export interface Wish {
  id: number;
  user_id: number;
  title: string;
  content: string;
  category: string;
  status: 'pending' | 'accepted' | 'rejected' | 'done' | string;
  created_at?: string;
  updated_at?: string;
}

export interface CreateWishPayload {
  title: string;
  content: string;
  category?: string;
}

export interface UpdateWishPayload {
  title?: string;
  content?: string;
  category?: string;
  status?: string;
}
