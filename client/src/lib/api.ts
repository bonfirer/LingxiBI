const BASE = '/api';

import { clearEmbedToken } from './embedToken';

import type {
  DataSource,
  CreateDataSourcePayload,
  UpdateDataSourcePayload,
  SchemaInfo,
  KnowledgeGraph,
  Conversation,
  Message,
  DataPool,
  Report,
  CreateReportPayload,
  ReportDataSource,
  ShareInfo,
  ReportGroup,
  MetricGroup,
  MetricPool,
  CreateMetricPayload,
  LLMConfig,
  UpdateLLMConfigPayload,
  QueryResult,
  KnowledgeEntry,
  CreateKnowledgeEntryPayload,
  SnapshotSchedule,
  CreateSnapshotSchedulePayload,
  UpdateSnapshotSchedulePayload,
  MetricSnapshot,
  SnapshotComparison,
  SmtpConfig,
  UpdateSmtpConfigPayload,
  FeishuConfig,
  UpdateFeishuConfigPayload,
  AlertRule,
  CreateAlertRulePayload,
  UpdateAlertRulePayload,
  AlertLog,
  AlertTemplate,
  GenerateAlertTemplatePayload,
} from './types';

// Re-export all types from the shared types module
export type {
  DataSource,
  CreateDataSourcePayload,
  UpdateDataSourcePayload,
  SchemaInfo,
  TableInfo,
  ColumnInfo,
  Relationship,
  GraphNode,
  GraphEdge,
  KnowledgeGraph,
  Conversation,
  Message,
  DataPool,
  Report,
  ReportConfig,
  VisConfig,
  CreateReportPayload,
  ReportDataSource,
  ShareInfo,
  ReportGroup,
  MetricGroup,
  MetricPool,
  CreateMetricPayload,
  LLMConfig,
  UpdateLLMConfigPayload,
  QueryResult,
  KnowledgeEntry,
  CreateKnowledgeEntryPayload,
  SnapshotSchedule,
  CreateSnapshotSchedulePayload,
  UpdateSnapshotSchedulePayload,
  MetricSnapshot,
  SnapshotComparison,
  SmtpConfig,
  UpdateSmtpConfigPayload,
  FeishuConfig,
  UpdateFeishuConfigPayload,
  AlertRule,
  CreateAlertRulePayload,
  UpdateAlertRulePayload,
  AlertLog,
  AlertTemplate,
  GenerateAlertTemplatePayload,
} from './types';

async function request<T>(url: string, options?: RequestInit): Promise<T> {
  const token = localStorage.getItem('token');
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...(token ? { Authorization: `Bearer ${token}` } : {}),
    ...(options?.headers as Record<string, string> || {}),
  };
  const res = await fetch(`${BASE}${url}`, {
    ...options,
    headers,
  });
  if (res.status === 401) {
    // Session expired/invalid — flag it and switch to the login screen via an
    // event instead of a jarring full-page reload. The login page surfaces a
    // "session expired" notice so the user understands what happened.
    localStorage.removeItem('token');
    try {
      sessionStorage.setItem('sessionExpired', '1');
    } catch { /* ignore */ }
    clearEmbedToken();
    window.dispatchEvent(new Event('auth-expired'));
    throw new Error('Unauthorized');
  }
  if (!res.ok) {
    const err = await res.text().catch(() => 'Unknown error');
    throw new Error(err || `HTTP ${res.status}`);
  }
  if (res.status === 204) return undefined as T;
  return res.json();
}

// ── Data Sources ──

export const datasourcesApi = {
  list: () => request<DataSource[]>('/datasources'),
  get: (id: number) => request<DataSource>(`/datasources/${id}`),
  create: (payload: CreateDataSourcePayload) =>
    request<DataSource>('/datasources', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: UpdateDataSourcePayload) =>
    request<DataSource>(`/datasources/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  remove: (id: number) =>
    request<void>(`/datasources/${id}`, { method: 'DELETE' }),
  testConnection: (id: number) =>
    request<{ status: string; message: string }>(`/datasources/${id}/test`, { method: 'POST' }),
  introspect: (id: number) =>
    request<SchemaInfo>(`/datasources/${id}/introspect`, { method: 'POST' }),
  getSchema: (id: number) =>
    request<SchemaInfo>(`/datasources/${id}/schema`),
  // Datasource-level access grants (admin only)
  listGrants: (id: number) =>
    request<number[]>(`/datasources/${id}/grants`),
  setGrants: (id: number, userIds: number[]) =>
    request<number[]>(`/datasources/${id}/grants`, { method: 'PUT', body: JSON.stringify({ user_ids: userIds }) }),
};

// ── Knowledge Graph ──

export const knowledgeGraphApi = {
  get: (dsId: number) => request<KnowledgeGraph>(`/knowledge-graph/${dsId}`),
  refresh: (dsId: number) =>
    request<KnowledgeGraph>(`/knowledge-graph/${dsId}/refresh`, { method: 'POST' }),
};

// ── Table Descriptions ──

export interface TableDescription {
  id: number;
  datasource_id: number;
  table_name: string;
  description: string;
  created_at?: string;
  updated_at?: string;
}

export const tableDescriptionsApi = {
  list: (dsId: number) => request<TableDescription[]>(`/datasources/${dsId}/table-descriptions`),
  upsert: (dsId: number, table_name: string, description: string) =>
    request<{ status: string }>(`/datasources/${dsId}/table-descriptions`, {
      method: 'POST',
      body: JSON.stringify({ table_name, description }),
    }),
};

export interface ColumnDescription {
  id: number;
  datasource_id: number;
  table_name: string;
  column_name: string;
  description: string;
  created_at?: string;
  updated_at?: string;
}

export const columnDescriptionsApi = {
  list: (dsId: number) => request<ColumnDescription[]>(`/datasources/${dsId}/column-descriptions`),
  upsert: (dsId: number, table_name: string, column_name: string, description: string) =>
    request<{ status: string }>(`/datasources/${dsId}/column-descriptions`, {
      method: 'POST',
      body: JSON.stringify({ table_name, column_name, description }),
    }),
};

// ── Conversations ──

export const conversationsApi = {
  list: () => request<Conversation[]>('/conversations'),
  create: () => request<Conversation>('/conversations', { method: 'POST' }),
  getMessages: (id: number) => request<Message[]>(`/conversations/${id}`),
  getStatus: (id: number) =>
    request<{ generation_status: string; generation_error: string | null }>(`/conversations/${id}/status`),
  delete: (id: number) =>
    request<void>(`/conversations/${id}`, { method: 'DELETE' }),
};

// ── Query ──

export const queryApi = {
  execute: (sql: string, datasource_id: number) =>
    request<QueryResult>('/query/execute', { method: 'POST', body: JSON.stringify({ sql, datasource_id }) }),
  getPool: (poolId: number) => request<DataPool>(`/query/${poolId}`),
};

// ── Reports ──

export const reportsApi = {
  list: () => request<Report[]>('/reports'),
  get: (id: number) => request<Report>(`/reports/${id}`),
  create: (payload: CreateReportPayload) =>
    request<Report>('/reports', { method: 'POST', body: JSON.stringify(payload) }),
  renderAI: (id: number, prompt: string, themeId?: number | null, signal?: AbortSignal) =>
    request<Report>(`/reports/${id}/render`, {
      method: 'POST',
      body: JSON.stringify({ prompt, theme_id: themeId ?? null }),
      signal,
    }),
  getStatus: (id: number) =>
    request<{ status: string; error: string | null; updated_at: string | null }>(`/reports/${id}/status`),
  delete: (id: number) =>
    request<void>(`/reports/${id}`, { method: 'DELETE' }),
  move: (id: number, groupId: number | null) =>
    request<Report>(`/reports/${id}/move`, { method: 'PUT', body: JSON.stringify({ group_id: groupId }) }),
  publish: (id: number, status: 'draft' | 'published') =>
    request<Report>(`/reports/${id}/publish`, { method: 'PUT', body: JSON.stringify({ status }) }),
  rollback: (id: number) =>
    request<Report>(`/reports/${id}/rollback`, { method: 'POST' }),
  share: (id: number, isPublic: boolean) =>
    request<ShareInfo>(`/reports/${id}/share`, { method: 'POST', body: JSON.stringify({ public: isPublic }) }),
  // Report datasources
  listDatasources: (reportId: number) =>
    request<ReportDataSource[]>(`/reports/${reportId}/datasources`),
  addDatasource: (reportId: number, payload: { metric_id?: number | null; name: string; sql_query: string; datasource_id: number }) =>
    request<ReportDataSource>(`/reports/${reportId}/datasources`, { method: 'POST', body: JSON.stringify(payload) }),
  removeDatasource: (reportId: number, dsId: number) =>
    request<void>(`/reports/${reportId}/datasources/${dsId}`, { method: 'DELETE' }),
  refreshDatasource: (reportId: number, dsId: number) =>
    request<ReportDataSource>(`/reports/${reportId}/datasources/${dsId}/refresh`, { method: 'POST' }),
  updateRefreshInterval: (reportId: number, interval: number) =>
    request<Report>(`/reports/${reportId}/refresh-interval`, { method: 'PUT', body: JSON.stringify({ refresh_interval: interval }) }),
  updateStyle: (reportId: number, styleKey: string | null) =>
    request<Report>(`/reports/${reportId}/style`, { method: 'PUT', body: JSON.stringify({ style_key: styleKey }) }),
};

// ── Report Themes (user-curated, reusable dashboard styles) ──

export interface ReportTheme {
  id: number;
  name: string;
  description: string;
  style_prompt?: string | null;
  emoji: string;
  source_report_id?: number | null;
  created_at?: string;
  updated_at?: string;
}

export interface CreateReportThemePayload {
  name: string;
  description?: string;
  style_prompt?: string | null;
  emoji?: string;
  source_report_id?: number | null;
  sample_html?: string | null;
}

export const reportThemesApi = {
  list: () => request<ReportTheme[]>('/report-themes'),
  create: (payload: CreateReportThemePayload) =>
    request<ReportTheme>('/report-themes', { method: 'POST', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/report-themes/${id}`, { method: 'DELETE' }),
};

// ── AI Data Summary ──

export interface DataSummary {
  headline: string;
  highlights: string[];
  trends: string[];
  anomalies: string[];
  recommendations: string[];
}

export interface ReportSummaryResponse {
  summary: DataSummary | null;
  model?: string;
  lang?: string;
  updated_at?: string | null;
}

export const reportSummaryApi = {
  get: (reportId: number) =>
    request<ReportSummaryResponse>(`/reports/${reportId}/summary`),
  generate: (reportId: number, lang?: string) =>
    request<ReportSummaryResponse>(`/reports/${reportId}/summary`, {
      method: 'POST',
      body: JSON.stringify({ lang: lang ?? 'zh' }),
    }),
  ask: (reportId: number, question: string, history: { role: string; content: string }[], lang?: string) =>
    request<{ answer: string }>(`/reports/${reportId}/ask`, {
      method: 'POST',
      body: JSON.stringify({ question, history, lang: lang ?? 'zh' }),
    }),
};

// ── Report Groups ──

export const reportGroupsApi = {
  list: () => request<ReportGroup[]>('/report-groups'),
  create: (payload: { name: string; description?: string }) =>
    request<ReportGroup>('/report-groups', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: { name?: string; description?: string; sort_order?: number }) =>
    request<ReportGroup>(`/report-groups/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/report-groups/${id}`, { method: 'DELETE' }),
};

// ── Metric Groups ──

export const metricGroupsApi = {
  list: () => request<MetricGroup[]>('/metric-groups'),
  create: (payload: { name: string; description?: string }) =>
    request<MetricGroup>('/metric-groups', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: { name?: string; description?: string; sort_order?: number }) =>
    request<MetricGroup>(`/metric-groups/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/metric-groups/${id}`, { method: 'DELETE' }),
};

// ── Metric Pools ──

export const metricsApi = {
  list: () => request<MetricPool[]>('/metrics'),
  get: (id: number) => request<MetricPool>(`/metrics/${id}`),
  create: (payload: CreateMetricPayload) =>
    request<MetricPool>('/metrics', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: { name?: string; description?: string; sql_query?: string; group_id?: number | null }) =>
    request<MetricPool>(`/metrics/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/metrics/${id}`, { method: 'DELETE' }),
  refresh: (id: number) =>
    request<MetricPool>(`/metrics/${id}/refresh`, { method: 'POST' }),
  move: (id: number, groupId: number | null) =>
    request<MetricPool>(`/metrics/${id}/move`, { method: 'PUT', body: JSON.stringify({ group_id: groupId }) }),
};

// ── LLM Config ──

export const llmConfigApi = {
  get: () => request<LLMConfig>('/llm/config'),
  update: (payload: UpdateLLMConfigPayload) =>
    request<LLMConfig>('/llm/config', { method: 'PUT', body: JSON.stringify(payload) }),
  test: () =>
    request<{ status: string; message: string }>('/llm/config/test', { method: 'POST' }),
};

// ── Knowledge Base ──

export const knowledgeBaseApi = {
  list: () => request<KnowledgeEntry[]>('/knowledge-base'),
  listByDatasource: (dsId: number) => request<KnowledgeEntry[]>(`/knowledge-base/datasource/${dsId}`),
  create: (payload: CreateKnowledgeEntryPayload) =>
    request<KnowledgeEntry>('/knowledge-base', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: { title?: string; content?: string; category?: string; confidence?: string }) =>
    request<KnowledgeEntry>(`/knowledge-base/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/knowledge-base/${id}`, { method: 'DELETE' }),
};

// ── Metric Snapshots ──

export const snapshotsApi = {
  // Schedules
  listSchedules: () => request<SnapshotSchedule[]>('/snapshot-schedules'),
  getSchedule: (metricId: number) => request<SnapshotSchedule>(`/metrics/${metricId}/schedule`),
  createSchedule: (payload: CreateSnapshotSchedulePayload) =>
    request<SnapshotSchedule>('/snapshot-schedules', { method: 'POST', body: JSON.stringify(payload) }),
  updateSchedule: (id: number, payload: UpdateSnapshotSchedulePayload) =>
    request<SnapshotSchedule>(`/snapshot-schedules/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  deleteSchedule: (id: number) =>
    request<void>(`/snapshot-schedules/${id}`, { method: 'DELETE' }),

  // Snapshot data
  listSnapshots: (metricId: number, params?: { period_type?: string; limit?: number }) => {
    const searchParams = new URLSearchParams();
    if (params?.period_type) searchParams.set('period_type', params.period_type);
    if (params?.limit) searchParams.set('limit', String(params.limit));
    const qs = searchParams.toString();
    return request<MetricSnapshot[]>(`/metrics/${metricId}/snapshots${qs ? `?${qs}` : ''}`);
  },
  takeSnapshot: (metricId: number) =>
    request<MetricSnapshot>(`/metrics/${metricId}/snapshots`, { method: 'POST' }),
  deleteSnapshot: (metricId: number, snapshotId: number) =>
    request<void>(`/metrics/${metricId}/snapshots/${snapshotId}`, { method: 'DELETE' }),
  compare: (metricId: number, params: { period_type: string; current_key: string; previous_key: string }) => {
    const searchParams = new URLSearchParams({
      period_type: params.period_type,
      current_key: params.current_key,
      previous_key: params.previous_key,
    });
    return request<SnapshotComparison>(`/metrics/${metricId}/snapshots/compare?${searchParams.toString()}`);
  },
};

// ── Email Alerts ──

export const alertsApi = {
  // SMTP config
  getSmtp: () => request<SmtpConfig>('/alerts/smtp'),
  updateSmtp: (payload: UpdateSmtpConfigPayload) =>
    request<SmtpConfig>('/alerts/smtp', { method: 'PUT', body: JSON.stringify(payload) }),
  testSmtp: (to: string) =>
    request<{ status: string; message: string }>('/alerts/smtp/test', {
      method: 'POST',
      body: JSON.stringify({ to }),
    }),

  // Feishu config
  getFeishu: () => request<FeishuConfig>('/alerts/feishu'),
  updateFeishu: (payload: UpdateFeishuConfigPayload) =>
    request<FeishuConfig>('/alerts/feishu', { method: 'PUT', body: JSON.stringify(payload) }),
  testFeishu: () =>
    request<{ status: string; message: string }>('/alerts/feishu/test', { method: 'POST' }),

  // Alert rules
  listRules: () => request<AlertRule[]>('/alerts/rules'),
  getRule: (id: number) => request<AlertRule>(`/alerts/rules/${id}`),
  createRule: (payload: CreateAlertRulePayload) =>
    request<AlertRule>('/alerts/rules', { method: 'POST', body: JSON.stringify(payload) }),
  updateRule: (id: number, payload: UpdateAlertRulePayload) =>
    request<AlertRule>(`/alerts/rules/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  deleteRule: (id: number) =>
    request<void>(`/alerts/rules/${id}`, { method: 'DELETE' }),
  triggerRule: (id: number) =>
    request<{ triggered: boolean; status: string; evaluated_value: number | null; message: string }>(
      `/alerts/rules/${id}/trigger`,
      { method: 'POST' },
    ),
  testRule: (id: number, recipients?: string[]) =>
    request<{ status: string; message: string }>(`/alerts/rules/${id}/test`, {
      method: 'POST',
      body: JSON.stringify({ recipients: recipients ?? null }),
    }),

  // AI template + logs
  generateTemplate: (payload: GenerateAlertTemplatePayload) =>
    request<AlertTemplate>('/alerts/generate-template', { method: 'POST', body: JSON.stringify(payload) }),
  listLogs: (params?: { rule_id?: number; limit?: number }) => {
    const sp = new URLSearchParams();
    if (params?.rule_id) sp.set('rule_id', String(params.rule_id));
    if (params?.limit) sp.set('limit', String(params.limit));
    const qs = sp.toString();
    return request<AlertLog[]>(`/alerts/logs${qs ? `?${qs}` : ''}`);
  },
};

// ── Auth / current user ──

export interface CurrentUser {
  id: number;
  username: string;
  display_name?: string | null;
  role?: string | null;
}

export const authApi = {
  me: () => request<CurrentUser>('/auth/me'),
};

// ── Users (admin only) ──

export interface User {
  id: number;
  username: string;
  display_name?: string | null;
  role?: string | null;
  created_at?: string;
}

export interface CreateUserPayload {
  username: string;
  password: string;
  display_name?: string;
  role?: string;
}

export interface UpdateUserPayload {
  display_name?: string;
  role?: string;
  password?: string;
}

export const usersApi = {
  list: () => request<User[]>('/users'),
  create: (payload: CreateUserPayload) =>
    request<User>('/users', { method: 'POST', body: JSON.stringify(payload) }),
  update: (id: number, payload: UpdateUserPayload) =>
    request<User>(`/users/${id}`, { method: 'PUT', body: JSON.stringify(payload) }),
  delete: (id: number) =>
    request<void>(`/users/${id}`, { method: 'DELETE' }),
};
