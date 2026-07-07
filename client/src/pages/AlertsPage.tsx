import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Bell,
  Plus,
  Trash,
  Gear,
  Sparkle,
  PaperPlaneTilt,
  Flask,
  Clock,
  CheckCircle,
  XCircle,
  WarningCircle,
  ChatCircleDots,
  List,
} from '@phosphor-icons/react';
import { useAlertStore } from '../stores/alertStore';
import { metricsApi, alertsApi } from '../lib/api';
import type { MetricPool, AlertRule, AlertLog, AlertOperator } from '../lib/types';
import SmtpModal from '../components/SmtpModal';
import FeishuModal from '../components/FeishuModal';

const OPERATORS: { value: AlertOperator; label: string }[] = [
  { value: 'gt', label: '> 大于' },
  { value: 'gte', label: '≥ 大于等于' },
  { value: 'lt', label: '< 小于' },
  { value: 'lte', label: '≤ 小于等于' },
  { value: 'eq', label: '= 等于' },
  { value: 'ne', label: '≠ 不等于' },
];

const SCHEDULE_TYPES = ['hourly', 'daily', 'weekly', 'monthly', 'cron'];

export default function AlertsPage() {
  const { t, i18n } = useTranslation();
  const { rules, logs, smtp, feishu, loading, fetchRules, deleteRule, updateRule, fetchSmtp, fetchFeishu, fetchLogs } = useAlertStore();

  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [showSmtp, setShowSmtp] = useState(false);
  const [showFeishu, setShowFeishu] = useState(false);
  const [creating, setCreating] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<number | null>(null);
  // Mobile: the rule list is an overlay drawer toggled by a button.
  const [showList, setShowList] = useState(false);

  useEffect(() => {
    fetchRules();
    fetchSmtp();
    fetchFeishu();
    fetchLogs({ limit: 100 });
    metricsApi.list().then(setMetrics).catch(() => {});
  }, [fetchRules, fetchSmtp, fetchFeishu, fetchLogs]);

  const selectedRule = useMemo(
    () => rules.find((r) => r.id === selectedId) ?? null,
    [rules, selectedId],
  );

  const metricName = (id: number) => metrics.find((m) => m.id === id)?.name || `#${id}`;

  const confirmDelete = async () => {
    if (deleteTarget == null) return;
    await deleteRule(deleteTarget);
    if (selectedId === deleteTarget) setSelectedId(null);
    setDeleteTarget(null);
  };

  const toggleEnabled = async (rule: AlertRule) => {
    try {
      await updateRule(rule.id, { enabled: !rule.enabled });
    } catch { /* error surfaced via store */ }
  };

  return (
    <div className="flex h-full overflow-hidden relative">
      {/* Mobile overlay when the rule list drawer is open */}
      {showList && (
        <div
          className="md:hidden absolute inset-0 z-30 bg-black/60"
          onClick={() => setShowList(false)}
        />
      )}
      {/* Left: rule list */}
      <div
        className={`
          absolute md:static inset-y-0 left-0 z-40 w-72
          border-r border-obsidian-700 flex flex-col overflow-hidden flex-shrink-0 bg-obsidian-950
          transform transition-transform duration-200 md:transform-none
          ${showList ? 'translate-x-0' : '-translate-x-full md:translate-x-0'}
        `}
      >
        <div className="p-4 border-b border-obsidian-700">
          <div className="flex items-center justify-between mb-1">
            <h2 className="text-sm font-semibold text-gray-200 flex items-center gap-2">
              <Bell size={16} className="text-amber-500" />
              {t('alerts.title')}
            </h2>
            <div className="flex items-center gap-1">
              <button
                onClick={() => setShowSmtp(true)}
                title={t('alerts.smtpSettings')}
                className={`w-6 h-6 rounded flex items-center justify-center transition-colors ${
                  smtp?.enabled ? 'text-green-400 hover:bg-obsidian-800' : 'text-gray-500 hover:text-amber-500 hover:bg-obsidian-800'
                }`}
              >
                <Gear size={14} />
              </button>
              <button
                onClick={() => setShowFeishu(true)}
                title={t('alerts.feishuSettings')}
                className={`w-6 h-6 rounded flex items-center justify-center transition-colors ${
                  feishu?.enabled ? 'text-green-400 hover:bg-obsidian-800' : 'text-gray-500 hover:text-amber-500 hover:bg-obsidian-800'
                }`}
              >
                <ChatCircleDots size={14} />
              </button>
              <button
                onClick={() => { setCreating(true); setSelectedId(null); setShowList(false); }}
                title={t('alerts.newRule')}
                className="w-6 h-6 rounded flex items-center justify-center text-gray-400 hover:text-amber-500 hover:bg-obsidian-800 transition-colors"
              >
                <Plus size={14} />
              </button>
            </div>
          </div>
          <p className="text-[10px] text-gray-500">{t('alerts.description')}</p>
          {!smtp?.enabled && (
            <div className="mt-2 text-[10px] text-amber-500/80 bg-amber-500/10 rounded px-2 py-1.5 flex items-center gap-1">
              <WarningCircle size={11} />
              {t('alerts.smtpNotConfigured')}
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto scrollbar-thin p-2 space-y-1">
          {rules.length === 0 && !loading && (
            <div className="text-center py-8 px-4">
              <Bell size={32} className="mx-auto text-gray-600 mb-2" />
              <p className="text-xs text-gray-500">{t('alerts.noRules')}</p>
              <p className="text-[10px] text-gray-600 mt-1">{t('alerts.noRulesHint')}</p>
            </div>
          )}
          {rules.map((rule) => (
            <div
              key={rule.id}
              onClick={() => { setSelectedId(rule.id); setCreating(false); setShowList(false); }}
              className={`p-2.5 rounded-lg cursor-pointer transition-colors border ${
                selectedId === rule.id && !creating
                  ? 'bg-amber-500/5 border-amber-500/20'
                  : 'border-transparent hover:bg-obsidian-800'
              }`}
            >
              <div className="flex items-center justify-between">
                <span className="text-xs text-gray-200 font-medium truncate">{rule.name}</span>
                <button
                  onClick={(e) => { e.stopPropagation(); setDeleteTarget(rule.id); }}
                  className="w-5 h-5 rounded flex items-center justify-center text-gray-500 hover:text-red-400"
                >
                  <Trash size={10} />
                </button>
              </div>
              <div className="text-[10px] text-gray-500 mt-0.5 truncate">{metricName(rule.metric_pool_id)}</div>
              <div className="flex items-center gap-2 mt-1">
                <span className="text-[9px] px-1.5 py-0.5 rounded bg-obsidian-700 text-gray-400">
                  {t(`alerts.schedule.${rule.schedule_type}`)}
                </span>
                <button
                  onClick={(e) => { e.stopPropagation(); toggleEnabled(rule); }}
                  title={rule.enabled ? t('alerts.pauseHint') : t('alerts.resumeHint')}
                  className={`px-1.5 py-0.5 rounded text-[9px] font-medium transition-colors ${
                    rule.enabled
                      ? 'bg-green-500/15 text-green-400 hover:bg-green-500/25'
                      : 'bg-gray-500/15 text-gray-500 hover:bg-gray-500/25'
                  }`}
                >
                  {rule.enabled ? t('alerts.running') : t('alerts.paused')}
                </button>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Right: editor or logs */}
      <div className="flex-1 flex flex-col overflow-hidden min-w-0">
        {/* Mobile-only bar to open the rule list drawer */}
        <div className="md:hidden flex items-center gap-2 px-3 py-2 border-b border-obsidian-700 flex-shrink-0">
          <button
            onClick={() => setShowList(true)}
            className="w-8 h-8 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-200 hover:bg-obsidian-800"
            aria-label={t('alerts.title')}
          >
            <List size={18} />
          </button>
          <span className="text-sm font-semibold text-gray-200">{t('alerts.title')}</span>
        </div>
        {creating || selectedRule ? (
          <RuleEditor
            key={selectedRule?.id ?? 'new'}
            rule={selectedRule}
            metrics={metrics}
            lang={i18n.language}
            onSaved={(r) => { setCreating(false); setSelectedId(r.id); fetchRules(); }}
            onCancel={() => { setCreating(false); }}
            onRefreshLogs={() => fetchLogs({ limit: 100 })}
          />
        ) : (
          <div className="flex-1 flex items-center justify-center">
            <div className="text-center">
              <Bell size={48} className="mx-auto text-gray-700 mb-3" />
              <p className="text-sm text-gray-500">{t('alerts.selectOrCreate')}</p>
            </div>
          </div>
        )}

        {/* Recent logs strip */}
        {!creating && (
          <LogsPanel logs={selectedRule ? logs.filter((l) => l.alert_rule_id === selectedRule.id) : logs} t={t} />
        )}
      </div>

      {showSmtp && <SmtpModal onClose={() => setShowSmtp(false)} />}
      {showFeishu && <FeishuModal onClose={() => setShowFeishu(false)} />}

      {deleteTarget != null && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setDeleteTarget(null)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-sm font-semibold text-gray-100 mb-2">{t('alerts.deleteConfirm.title')}</h3>
            <p className="text-xs text-gray-400 mb-4">{t('alerts.deleteConfirm.message')}</p>
            <div className="flex items-center justify-end gap-2">
              <button onClick={() => setDeleteTarget(null)} className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700">
                {t('common.cancel')}
              </button>
              <button onClick={confirmDelete} className="text-xs text-white bg-red-600 hover:bg-red-500 px-3 py-1.5 rounded-md">
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Logs panel ──
function LogsPanel({ logs, t }: { logs: AlertLog[]; t: (k: string) => string }) {
  if (logs.length === 0) return null;
  const statusIcon = (status: string) => {
    if (status === 'sent') return <CheckCircle size={11} className="text-green-400" weight="fill" />;
    if (status === 'failed') return <XCircle size={11} className="text-red-400" weight="fill" />;
    if (status === 'skipped') return <Clock size={11} className="text-amber-400" weight="fill" />;
    return <WarningCircle size={11} className="text-gray-500" />;
  };
  return (
    <div className="border-t border-obsidian-700 h-44 flex flex-col flex-shrink-0">
      <div className="px-4 py-2 border-b border-obsidian-700/50">
        <span className="text-[11px] font-semibold text-gray-400 uppercase tracking-wider">{t('alerts.recentLogs')}</span>
      </div>
      <div className="flex-1 overflow-y-auto scrollbar-thin">
        {logs.slice(0, 50).map((log) => (
          <div key={log.id} className="px-4 py-1.5 flex items-center gap-2 border-b border-obsidian-800/50 hover:bg-obsidian-800/30">
            {statusIcon(log.status)}
            <span className="text-[10px] text-gray-500 w-32 flex-shrink-0">
              {log.created_at ? new Date(log.created_at).toLocaleString() : ''}
            </span>
            <span className={`text-[10px] px-1.5 py-0.5 rounded flex-shrink-0 ${
              log.status === 'sent' ? 'bg-green-500/10 text-green-400'
              : log.status === 'failed' ? 'bg-red-500/10 text-red-400'
              : 'bg-gray-500/10 text-gray-400'
            }`}>
              {t(`alerts.logStatus.${log.status}`)}
            </span>
            <span className="text-[10px] text-gray-400 truncate flex-1">
              {log.evaluated_value != null && <span className="text-gray-500 mr-2">值: {log.evaluated_value}</span>}
              {log.error || log.message}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Rule editor ──
function RuleEditor({
  rule,
  metrics,
  lang,
  onSaved,
  onCancel,
  onRefreshLogs,
}: {
  rule: AlertRule | null;
  metrics: MetricPool[];
  lang: string;
  onSaved: (r: AlertRule) => void;
  onCancel: () => void;
  onRefreshLogs: () => void;
}) {
  const { t } = useTranslation();
  const { createRule, updateRule } = useAlertStore();

  const [name, setName] = useState(rule?.name ?? '');
  const [metricId, setMetricId] = useState<number | ''>(rule?.metric_pool_id ?? '');
  const [conditionColumn, setConditionColumn] = useState<string>(rule?.condition_column ?? '');
  const [operator, setOperator] = useState<AlertOperator>(rule?.operator ?? 'gt');
  const [threshold, setThreshold] = useState<string>(rule ? String(rule.threshold) : '0');
  const [recipients, setRecipients] = useState<string>((rule?.recipients ?? []).join(', '));
  const [scheduleType, setScheduleType] = useState(rule?.schedule_type ?? 'daily');
  const [cronExpr, setCronExpr] = useState(rule?.cron_expr ?? '');
  const [cooldown, setCooldown] = useState<string>(rule ? String(rule.cooldown_minutes) : '0');
  const [includeExcel, setIncludeExcel] = useState(rule?.include_excel ?? true);
  const [notifyFeishu, setNotifyFeishu] = useState(rule?.notify_feishu ?? false);
  const [subject, setSubject] = useState(rule?.subject_template ?? '');
  const [body, setBody] = useState(rule?.body_template ?? '');
  const [enabled, setEnabled] = useState(rule?.enabled ?? true);

  const [busy, setBusy] = useState(false);
  const [aiBusy, setAiBusy] = useState(false);
  const [aiInstructions, setAiInstructions] = useState('');
  const [msg, setMsg] = useState<{ type: 'ok' | 'err'; text: string } | null>(null);
  const [previewMode, setPreviewMode] = useState(true);

  // Columns available from the selected metric's cached result. The metrics
  // list no longer carries `result_cache`, so fetch the single metric to read
  // its column names.
  const [columns, setColumns] = useState<string[]>([]);
  useEffect(() => {
    if (!metricId) { setColumns([]); return; }
    let cancelled = false;
    metricsApi.get(metricId as number)
      .then((full) => {
        if (cancelled) return;
        const cache = full.result_cache;
        if (Array.isArray(cache) && cache.length > 0 && typeof cache[0] === 'object' && cache[0]) {
          setColumns(Object.keys(cache[0] as Record<string, unknown>));
        } else {
          setColumns([]);
        }
      })
      .catch(() => { if (!cancelled) setColumns([]); });
    return () => { cancelled = true; };
  }, [metricId]);

  const parsedRecipients = () =>
    recipients.split(/[,;\n]/).map((s) => s.trim()).filter(Boolean);

  const flash = (type: 'ok' | 'err', text: string) => {
    setMsg({ type, text });
    setTimeout(() => setMsg(null), 4000);
  };

  const handleSave = async (): Promise<AlertRule | null> => {
    if (!name.trim() || !metricId) {
      flash('err', t('alerts.validation.nameMetric'));
      return null;
    }
    if (parsedRecipients().length === 0 && !notifyFeishu) {
      flash('err', t('alerts.validation.channel'));
      return null;
    }
    setBusy(true);
    try {
      const payload = {
        name: name.trim(),
        condition_column: conditionColumn || null,
        operator,
        threshold: parseFloat(threshold) || 0,
        recipients: parsedRecipients(),
        schedule_type: scheduleType,
        cron_expr: scheduleType === 'cron' ? cronExpr : null,
        subject_template: subject,
        body_template: body || null,
        include_excel: includeExcel,
        notify_feishu: notifyFeishu,
        cooldown_minutes: parseInt(cooldown) || 0,
        enabled,
      };
      let saved: AlertRule;
      if (rule) {
        saved = await updateRule(rule.id, payload);
      } else {
        saved = await createRule({ metric_pool_id: metricId as number, ...payload });
      }
      flash('ok', t('alerts.saved'));
      onSaved(saved);
      return saved;
    } catch (e) {
      flash('err', (e as Error).message);
      return null;
    } finally {
      setBusy(false);
    }
  };

  const handleGenerateTemplate = async () => {
    if (!metricId) { flash('err', t('alerts.validation.nameMetric')); return; }
    setAiBusy(true);
    try {
      const tpl = await alertsApi.generateTemplate({
        metric_pool_id: metricId as number,
        operator,
        threshold: parseFloat(threshold) || 0,
        condition_column: conditionColumn || null,
        instructions: aiInstructions || undefined,
        lang,
      });
      setSubject(tpl.subject_template);
      setBody(tpl.body_template);
      flash('ok', t('alerts.aiGenerated'));
    } catch (e) {
      flash('err', (e as Error).message);
    } finally {
      setAiBusy(false);
    }
  };

  const handleTrigger = async () => {
    // Always persist the current form first so toggles like "push to Feishu"
    // take effect — the backend re-reads the rule from the DB when running it.
    const saved = await handleSave();
    if (!saved) return;
    setBusy(true);
    try {
      const res = await alertsApi.triggerRule(saved.id);
      flash(res.status === 'failed' ? 'err' : 'ok', `${t(`alerts.logStatus.${res.status}`)} — ${res.message}`);
      onRefreshLogs();
    } catch (e) {
      flash('err', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const handleTest = async () => {
    // Persist first (see handleTrigger) so the test uses the latest settings.
    const saved = await handleSave();
    if (!saved) return;
    setBusy(true);
    try {
      const res = await alertsApi.testRule(saved.id, parsedRecipients());
      // A "sent" status can still hide a failed channel — surface those.
      const partial = /partial failures/i.test(res.message);
      flash(partial ? 'err' : 'ok', res.message);
      onRefreshLogs();
    } catch (e) {
      flash('err', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const labelCls = 'block text-[11px] font-medium text-gray-400 mb-1';
  const inputCls = 'w-full bg-obsidian-900 border border-obsidian-700 rounded-md px-2.5 py-1.5 text-xs text-gray-200 focus:border-amber-500/50 focus:outline-none';

  return (
    <div className="flex-1 overflow-y-auto scrollbar-thin">
      <div className="p-5 max-w-2xl mx-auto space-y-4">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-gray-100">
            {rule ? t('alerts.editRule') : t('alerts.newRule')}
          </h3>
          {msg && (
            <span className={`text-[11px] px-2 py-1 rounded ${msg.type === 'ok' ? 'bg-green-500/10 text-green-400' : 'bg-red-500/10 text-red-400'}`}>
              {msg.text}
            </span>
          )}
        </div>

        {/* Basics */}
        <div className="grid grid-cols-2 gap-3">
          <div className="col-span-2">
            <label className={labelCls}>{t('alerts.fields.name')}</label>
            <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} placeholder={t('alerts.fields.namePlaceholder')} />
          </div>
          <div>
            <label className={labelCls}>{t('alerts.fields.metric')}</label>
            <select className={inputCls} value={metricId} onChange={(e) => { setMetricId(e.target.value ? Number(e.target.value) : ''); setConditionColumn(''); }} disabled={!!rule}>
              <option value="">{t('alerts.fields.selectMetric')}</option>
              {metrics.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
            </select>
          </div>
          <div>
            <label className={labelCls}>{t('alerts.fields.column')}</label>
            <select className={inputCls} value={conditionColumn} onChange={(e) => setConditionColumn(e.target.value)}>
              <option value="">{t('alerts.fields.firstNumeric')}</option>
              {columns.map((c) => <option key={c} value={c}>{c}</option>)}
            </select>
          </div>
        </div>

        {/* Condition */}
        <div>
          <label className={labelCls}>{t('alerts.fields.condition')}</label>
          <div className="flex items-center gap-2">
            <span className="text-xs text-gray-500 whitespace-nowrap flex-shrink-0">{t('alerts.fields.conditionPrefix')}</span>
            <select className={`${inputCls} w-40`} value={operator} onChange={(e) => setOperator(e.target.value as AlertOperator)}>
              {OPERATORS.map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}
            </select>
            <input className={`${inputCls} w-32`} type="number" value={threshold} onChange={(e) => setThreshold(e.target.value)} />
          </div>
        </div>

        {/* Schedule */}
        <div className="grid grid-cols-3 gap-3">
          <div>
            <label className={labelCls}>{t('alerts.fields.schedule')}</label>
            <select className={inputCls} value={scheduleType} onChange={(e) => setScheduleType(e.target.value)}>
              {SCHEDULE_TYPES.map((s) => <option key={s} value={s}>{t(`alerts.schedule.${s}`)}</option>)}
            </select>
          </div>
          {scheduleType === 'cron' && (
            <div>
              <label className={labelCls}>{t('alerts.fields.cron')}</label>
              <input className={inputCls} value={cronExpr} onChange={(e) => setCronExpr(e.target.value)} placeholder="*/5 * * * * 或 30s" />
            </div>
          )}
          <div>
            <label className={labelCls}>{t('alerts.fields.cooldown')}</label>
            <input className={inputCls} type="number" value={cooldown} onChange={(e) => setCooldown(e.target.value)} />
          </div>
        </div>

        {/* Recipients */}
        <div>
          <label className={labelCls}>{t('alerts.fields.recipients')}</label>
          <textarea className={`${inputCls} h-16 resize-none`} value={recipients} onChange={(e) => setRecipients(e.target.value)} placeholder="alice@example.com, bob@example.com" />
        </div>

        {/* Options */}
        <div className="flex items-center gap-4">
          <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
            <input type="checkbox" checked={includeExcel} onChange={(e) => setIncludeExcel(e.target.checked)} className="accent-amber-500" />
            {t('alerts.fields.includeExcel')}
          </label>
          <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
            <input type="checkbox" checked={notifyFeishu} onChange={(e) => setNotifyFeishu(e.target.checked)} className="accent-amber-500" />
            {t('alerts.fields.notifyFeishu')}
          </label>
          <label className="flex items-center gap-2 text-xs text-gray-300 cursor-pointer">
            <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="accent-amber-500" />
            {t('alerts.fields.enabled')}
          </label>
        </div>

        {/* Template + AI */}
        <div className="border border-obsidian-700 rounded-lg p-3 space-y-3 bg-obsidian-900/40">
          <div className="flex items-center justify-between">
            <span className="text-[11px] font-semibold text-gray-300 flex items-center gap-1.5">
              <Sparkle size={13} className="text-amber-500" />
              {t('alerts.template')}
            </span>
            <button onClick={() => setPreviewMode(!previewMode)} className="text-[10px] text-gray-400 hover:text-amber-500">
              {previewMode ? t('alerts.editTemplate') : t('alerts.preview')}
            </button>
          </div>

          <div className="flex items-center gap-2">
            <input className={`${inputCls} flex-1`} value={aiInstructions} onChange={(e) => setAiInstructions(e.target.value)} placeholder={t('alerts.aiInstructionsPlaceholder')} />
            <button
              onClick={handleGenerateTemplate}
              disabled={aiBusy}
              className="px-3 py-1.5 rounded-md bg-amber-500/15 text-amber-500 text-[11px] font-medium hover:bg-amber-500/25 transition-colors disabled:opacity-50 flex items-center gap-1.5 whitespace-nowrap"
            >
              <Sparkle size={12} weight={aiBusy ? 'regular' : 'fill'} className={aiBusy ? 'animate-pulse' : ''} />
              {aiBusy ? t('alerts.generating') : t('alerts.aiGenerate')}
            </button>
          </div>

          <div>
            <label className={labelCls}>{t('alerts.fields.subject')}</label>
            <input className={inputCls} value={subject} onChange={(e) => setSubject(e.target.value)} placeholder="[预警] {{metric_name}} 已达 {{value}}" />
          </div>

          <div>
            <label className={labelCls}>{t('alerts.fields.body')}</label>
            {previewMode ? (
              <div className="bg-white rounded-md p-2 max-h-80 overflow-auto" dangerouslySetInnerHTML={{ __html: body || '<p style="color:#999">（空模板，发送时使用默认模板）</p>' }} />
            ) : (
              <textarea className={`${inputCls} h-48 resize-none font-mono text-[11px]`} value={body} onChange={(e) => setBody(e.target.value)} placeholder={t('alerts.bodyPlaceholder')} />
            )}
            <p className="text-[10px] text-gray-600 mt-1">{t('alerts.placeholderHint')}</p>
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2 pt-1">
          <button onClick={handleSave} disabled={busy} className="px-4 py-1.5 rounded-md bg-amber-500 text-[#08080c] text-xs font-semibold hover:bg-amber-400 transition-colors disabled:opacity-50">
            {t('common.save')}
          </button>
          <button onClick={handleTrigger} disabled={busy} className="px-3 py-1.5 rounded-md bg-obsidian-800 text-gray-200 text-xs hover:bg-obsidian-700 transition-colors disabled:opacity-50 flex items-center gap-1.5">
            <PaperPlaneTilt size={12} />
            {t('alerts.triggerNow')}
          </button>
          <button onClick={handleTest} disabled={busy} className="px-3 py-1.5 rounded-md bg-obsidian-800 text-gray-200 text-xs hover:bg-obsidian-700 transition-colors disabled:opacity-50 flex items-center gap-1.5">
            <Flask size={12} />
            {t('alerts.sendTest')}
          </button>
          {!rule && (
            <button onClick={onCancel} className="px-3 py-1.5 rounded-md text-gray-400 text-xs hover:text-gray-200 border border-obsidian-700">
              {t('common.cancel')}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
