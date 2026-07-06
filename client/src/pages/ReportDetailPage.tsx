import { useEffect, useState, useCallback, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  ArrowLeft, Sparkle, PaperPlaneRight, ShareNetwork, Upload,
  Plus, Database, Star, X, Stop, Clock, ArrowClockwise, CaretDown, ArrowCounterClockwise,
  Desktop, DeviceMobile, ClockCounterClockwise, Trash, Palette, Lightbulb, ChatCircle, Funnel, Bug,
} from '@phosphor-icons/react';
import {
  reportsApi, metricsApi, metricGroupsApi, reportThemesApi, reportSummaryApi,
  type Report, type ReportDataSource, type MetricPool, type MetricGroup, type ReportTheme, type DataSummary,
} from '../lib/api';
import { ErrorBanner } from '../components/ui';
import { DatasourceFilters } from '../components/DatasourceFilters';
import { ReportFilters } from '../components/ReportFilters';
import { ReportDebugPanel } from '../components/ReportDebugPanel';
import { fetchEmbedToken, getCachedEmbedToken } from '../lib/embedToken';
import { toast } from '../stores/toastStore';

/// Build a report HTML/data URL with an auth token appended (for iframe loading).
/// Prefers a short-lived embed token so the long-lived session JWT never lands
/// in a URL; falls back to the session token if the embed token isn't ready.
function withToken(path: string): string {
  const token = getCachedEmbedToken() || localStorage.getItem('token') || '';
  const sep = path.includes('?') ? '&' : '?';
  return `${path}${sep}token=${encodeURIComponent(token)}`;
}

export default function ReportDetailPage() {
  const { t, i18n } = useTranslation();
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();

  const [report, setReport] = useState<Report | null>(null);
  const [datasources, setDatasources] = useState<ReportDataSource[]>([]);
  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [metricGroups, setMetricGroups] = useState<MetricGroup[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [aiInput, setAiInput] = useState('');
  const [aiLoading, setAiLoading] = useState(false);
  const [selectedStyleKey, setSelectedStyleKey] = useState<string | null>(null);
  const [showDsModal, setShowDsModal] = useState(false);
  const [showFilters, setShowFilters] = useState(false);
  const [showDebug, setShowDebug] = useState(false);
  const [showVersions, setShowVersions] = useState(false);
  const [versions, setVersions] = useState<{ id: number; version: number; prompt: string | null; style_key: string | null; created_at: string | null }[]>([]);
  const [showHistory, setShowHistory] = useState(false);
  const [history, setHistory] = useState<{ id: number; version: number; prompt: string | null; created_at: string | null }[]>([]);
  // Saved themes (user-curated reusable dashboard styles)
  const [themes, setThemes] = useState<ReportTheme[]>([]);
  const [showThemes, setShowThemes] = useState(false);
  const [showSaveTheme, setShowSaveTheme] = useState(false);
  // AI data-analysis summary
  const [showSummary, setShowSummary] = useState(false);
  const [summary, setSummary] = useState<DataSummary | null>(null);
  const [summaryUpdatedAt, setSummaryUpdatedAt] = useState<string | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(false);
  // AI Q&A over the report
  const [qaMessages, setQaMessages] = useState<{ role: 'user' | 'assistant'; content: string }[]>([]);
  const [qaInput, setQaInput] = useState('');
  const [qaLoading, setQaLoading] = useState(false);
  const qaEndRef = useRef<HTMLDivElement>(null);
  const [currentVersion, setCurrentVersion] = useState<number | null>(null);
  const [compareVersionId, setCompareVersionId] = useState<number | null>(null);
  const [deleteVersionTarget, setDeleteVersionTarget] = useState<{ id: number; version: number } | null>(null);
  const [refreshInterval, setRefreshInterval] = useState(1); // minutes, default 1
  const [deviceMode, setDeviceMode] = useState<'desktop' | 'mobile'>('desktop');
  const refreshTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const pollTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  // Mint a short-lived embed token on mount, then nudge a re-render so the
  // iframe src is rebuilt using it instead of the session JWT.
  const [, setEmbedTick] = useState(0);
  useEffect(() => {
    let active = true;
    fetchEmbedToken().then(() => {
      if (active) setEmbedTick((n) => n + 1);
    });
    return () => {
      active = false;
    };
  }, []);

  // Stop polling helper
  const stopPolling = useCallback(() => {
    if (pollTimerRef.current) { clearInterval(pollTimerRef.current); pollTimerRef.current = null; }
  }, []);

  // Fetch the latest version number for the title badge.
  const refreshVersion = useCallback(async (reportId: number) => {
    try {
      const res = await fetch(`/api/reports/${reportId}/versions`, {
        headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
      });
      if (res.ok) {
        const vs: { version: number }[] = await res.json();
        const max = vs.reduce((m, v) => Math.max(m, v.version), 0);
        setCurrentVersion(max > 0 ? max : null);
      }
    } catch {
      /* silent */
    }
  }, []);

  // Poll generation status until done/failed
  const startPolling = useCallback((reportId: number) => {
    stopPolling();
    setAiLoading(true);
    pollTimerRef.current = setInterval(async () => {
      try {
        const st = await reportsApi.getStatus(reportId);
        if (st.status === 'done') {
          stopPolling();
          setAiLoading(false);
          // Reload the report + iframe with fresh content
          const fresh = await reportsApi.get(reportId).catch(() => null);
          if (fresh) setReport(fresh);
          refreshVersion(reportId);
          if (iframeRef.current) {
            await fetchEmbedToken();
            iframeRef.current.src = withToken(`/api/reports/${reportId}/html?t=${Date.now()}`);
          }
        } else if (st.status === 'failed') {
          stopPolling();
          setAiLoading(false);
          toast.error(st.error || t('reportDetail.aiComposeFailed'));
        }
      } catch {
        // network hiccup — keep polling
      }
    }, 3000);
  }, [stopPolling, t, refreshVersion]);

  const fetchAll = useCallback(async () => {
    if (!id) return;
    try {
      setLoading(true);
      const [r, ds, m, mg] = await Promise.all([
        reportsApi.get(parseInt(id)),
        reportsApi.listDatasources(parseInt(id)),
        metricsApi.list(),
        metricGroupsApi.list(),
      ]);
      setReport(r);
      setDatasources(ds);
      setMetrics(m);
      setMetricGroups(mg);
      refreshVersion(r.id);
      if (r.refresh_interval != null) setRefreshInterval(r.refresh_interval);
      if (r.style_key) setSelectedStyleKey(r.style_key);
      // Resume polling if generation is already in progress (e.g. user navigated away and back)
      if (r.generation_status === 'generating') {
        startPolling(r.id);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load');
    } finally {
      setLoading(false);
    }
  }, [id, startPolling, refreshVersion]);

  useEffect(() => { fetchAll(); }, [fetchAll]);

  // Clean up polling on unmount
  useEffect(() => () => { if (pollTimerRef.current) clearInterval(pollTimerRef.current); }, []);

  // ── Auto-refresh timer ──
  // Sends the refresh interval to the iframe page via postMessage
  // The iframe page listens for this and adjusts its own setInterval
  useEffect(() => {
    if (refreshTimerRef.current) clearInterval(refreshTimerRef.current);

    // Tell iframe about the new interval
    const sendInterval = () => {
      if (iframeRef.current?.contentWindow) {
        iframeRef.current.contentWindow.postMessage(
          { type: 'setRefreshInterval', interval: refreshInterval * 60 * 1000 },
          '*'
        );
      }
    };

    // Send after iframe loads
    const timer = setTimeout(sendInterval, 1500);

    // Also set up parent-level reload as fallback
    if (refreshInterval > 0 && id) {
      refreshTimerRef.current = setInterval(() => {
        if (iframeRef.current?.contentWindow) {
          iframeRef.current.contentWindow.postMessage({ type: 'refreshNow' }, '*');
        }
      }, refreshInterval * 60 * 1000);
    }

    return () => {
      clearTimeout(timer);
      if (refreshTimerRef.current) clearInterval(refreshTimerRef.current);
    };
  }, [refreshInterval, id]);

  // ── AI Send ──
  const handleAISend = async () => {
    if (!report || !aiInput.trim()) return;
    let prompt = aiInput.trim();
    // If a style is selected, prefix with style context so AI maintains it
    if (selectedStyleKey) {
      const style = DOCK_STYLES.find(s => s.key === selectedStyleKey);
      if (style) {
        prompt = `[保持当前风格: ${style.label}] ${prompt}`;
      }
    }
    setAiInput('');
    try {
      await reportsApi.renderAI(report.id, prompt);
      startPolling(report.id);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('reportDetail.aiComposeFailed'));
    }
  };

  // Toggle the "ask history" popover, loading past prompts (report versions).
  const toggleHistory = async () => {
    const next = !showHistory;
    setShowHistory(next);
    if (next && report) {
      try {
        const res = await fetch(`/api/reports/${report.id}/versions`, {
          headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
        });
        if (res.ok) {
          const data = await res.json();
          setHistory(data);
          const max = (data as { version: number }[]).reduce((m, v) => Math.max(m, v.version), 0);
          setCurrentVersion(max > 0 ? max : null);
        }
      } catch {
        /* silent */
      }
    }
  };

  // Send a specific prompt directly (for quick action buttons)
  const handleAISendPrompt = async (prompt: string) => {
    if (!report) return;
    try {
      await reportsApi.renderAI(report.id, prompt);
      startPolling(report.id);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('reportDetail.aiComposeFailed'));
    }
  };

  // One-click "generate a full dashboard". If the user has typed a request,
  // treat it as the highest-priority requirement layered on the default layout;
  // otherwise fall back to the standard cockpit template. Honors the selected style.
  const handleQuickGenerate = () => {
    if (!report) return;
    const base = '生成一个专业的数据驾驶舱，顶部KPI卡片，中间趋势图和对比图，底部明细';
    const extra = aiInput.trim();
    let prompt = extra
      ? `${base}。\n\n【用户的具体要求 — 最高优先级，必须优先满足；与上述默认布局冲突时以此为准】：${extra}`
      : base;
    if (selectedStyleKey) {
      const style = DOCK_STYLES.find((s) => s.key === selectedStyleKey);
      if (style) prompt = `[保持当前风格: ${style.label}] ${prompt}`;
    }
    setAiInput('');
    handleAISendPrompt(prompt);
  };

  // ── Saved themes ──
  const loadThemes = useCallback(async () => {
    try {
      setThemes(await reportThemesApi.list());
    } catch { /* silent */ }
  }, []);

  const toggleThemes = async () => {
    const next = !showThemes;
    setShowThemes(next);
    if (next) await loadThemes();
  };

  // Generate the current report using a saved theme.
  const handleUseTheme = async (theme: ReportTheme) => {
    if (!report) return;
    setShowThemes(false);
    const extra = aiInput.trim();
    const prompt = extra
      ? `${t('reportDetail.themes.applyPrefix', { name: theme.name })} ${extra}`
      : t('reportDetail.themes.applyDefault', { name: theme.name });
    setAiInput('');
    try {
      await reportsApi.renderAI(report.id, prompt, theme.id);
      startPolling(report.id);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('reportDetail.aiComposeFailed'));
    }
  };

  const handleDeleteTheme = async (themeId: number) => {
    try {
      await reportThemesApi.delete(themeId);
      setThemes((prev) => prev.filter((th) => th.id !== themeId));
    } catch {
      toast.error(t('errors.deleteFailed'));
    }
  };

  // ── AI data-analysis summary ──
  const toggleSummary = async () => {
    const next = !showSummary;
    setShowSummary(next);
    if (next && report && !summary) {
      try {
        const res = await reportSummaryApi.get(report.id);
        if (res.summary) { setSummary(res.summary); setSummaryUpdatedAt(res.updated_at ?? null); }
      } catch { /* silent */ }
    }
  };

  const handleGenerateSummary = async () => {
    if (!report) return;
    setSummaryLoading(true);
    try {
      const res = await reportSummaryApi.generate(report.id, i18n.language);
      setSummary(res.summary);
      setSummaryUpdatedAt(res.updated_at ?? null);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('reportDetail.summary.failed'));
    } finally {
      setSummaryLoading(false);
    }
  };

  const handleAsk = async () => {
    if (!report || !qaInput.trim() || qaLoading) return;
    const question = qaInput.trim();
    const history = qaMessages.slice(-8);
    setQaMessages((prev) => [...prev, { role: 'user', content: question }]);
    setQaInput('');
    setQaLoading(true);
    // Scroll to bottom after the user message renders.
    setTimeout(() => qaEndRef.current?.scrollIntoView({ behavior: 'smooth' }), 50);
    try {
      const res = await reportSummaryApi.ask(report.id, question, history, i18n.language);
      setQaMessages((prev) => [...prev, { role: 'assistant', content: res.answer }]);
    } catch (e) {
      setQaMessages((prev) => [...prev, { role: 'assistant', content: `⚠️ ${e instanceof Error ? e.message : t('reportDetail.summary.failed')}` }]);
    } finally {
      setQaLoading(false);
      setTimeout(() => qaEndRef.current?.scrollIntoView({ behavior: 'smooth' }), 50);
    }
  };

  // Stop watching the generation (server task continues in background)
  const handleAIStop = () => {
    stopPolling();
    setAiLoading(false);
    toast.info(t('reportDetail.generationBackground'));
  };

  // ── Add metric as datasource ──
  const handleAddMetric = async (metric: MetricPool) => {
    if (!report) return;
    await reportsApi.addDatasource(report.id, {
      metric_id: metric.id, name: metric.name,
      sql_query: metric.sql_query, datasource_id: metric.datasource_id,
    }).catch(() => null);
    const ds = await reportsApi.listDatasources(report.id);
    setDatasources(ds);
  };

  // ── Add entire group ──
  const handleAddGroup = async (groupId: number) => {
    const groupMetrics = metrics.filter((m) => m.group_id === groupId && !datasources.some((d) => d.metric_id === m.id));
    for (const m of groupMetrics) {
      await handleAddMetric(m);
    }
    setShowDsModal(false);
  };

  // ── Actions ──
  const handlePublish = async () => {
    if (!report) return;
    const goPublish = report.status !== 'published';
    const updated = await reportsApi.publish(report.id, goPublish ? 'published' : 'draft').catch(() => null);
    if (updated) {
      setReport(updated);
      toast.success(goPublish ? t('reportDetail.publishedToast') : t('reportDetail.unpublishedToast'));
    } else {
      toast.error(t('errors.saveFailed'));
    }
  };

  const handleRollback = async () => {
    if (!report) return;
    const updated = await reportsApi.rollback(report.id).catch(() => null);
    if (updated) {
      setReport(updated);
      refreshVersion(report.id);
      await fetchEmbedToken();
      if (iframeRef.current) iframeRef.current.src = withToken(`/api/reports/${report.id}/html?t=${Date.now()}`);
      toast.success(t('reportDetail.rollbackSuccess'));
    }
  };

  const handleShare = async () => {
    if (!report) return;
    const info = await reportsApi.share(report.id, true).catch(() => null);
    if (info) {
      await navigator.clipboard.writeText(window.location.origin + `/api/share/${info.share_token}/html`);
      toast.success(t('reportDetail.shareCopied'));
    }
  };

  // ── Loading ──
  if (loading) return <div className="h-full flex items-center justify-center"><div className="w-4 h-4 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" /></div>;
  if (!report) return <div className="p-6"><ErrorBanner message="Report not found" onDismiss={() => navigate('/reports')} /></div>;

  const hasHtml = !!report.html_content;
  const iframeSrc = withToken(`/api/reports/${report.id}/html?t=${report.updated_at || ''}`);

  return (
    <div className="h-full flex flex-col">
      {/* ── Top Bar ── */}
      <div className="flex items-center px-5 py-2.5 border-b border-obsidian-700 flex-shrink-0">
        <div className="flex items-center gap-3 flex-1">
          <button onClick={() => navigate('/reports')} aria-label={t('common.back')} className="text-gray-500 hover:text-gray-300 transition-premium"><ArrowLeft size={16} /></button>
          <h1 className="text-sm font-bold text-gray-100">{report.title}</h1>
          {currentVersion != null && (
            <span className="text-[8px] bg-obsidian-700 text-gray-300 px-1.5 py-0.5 rounded-full font-mono font-medium" title={t('reportDetail.currentVersion')}>
              v{currentVersion}
            </span>
          )}
          {report.status === 'published' && <span className="text-[8px] bg-data-green/10 text-data-green px-1.5 py-0.5 rounded-full font-medium">LIVE</span>}
          {report.design_score && (
            <span className={`text-[8px] px-1.5 py-0.5 rounded-full font-mono font-medium ${
              report.design_score.total >= 80 ? 'bg-data-green/10 text-data-green' :
              report.design_score.total >= 60 ? 'bg-amber-500/10 text-amber-500' :
              'bg-gray-500/10 text-gray-400'
            }`} title={`布局:${report.design_score.layout} 配色:${report.design_score.color} 字体:${report.design_score.typography} 响应:${report.design_score.responsiveness} 图表:${report.design_score.data_viz}`}>
              ★ {report.design_score.total}
            </span>
          )}
        </div>
        {/* Center: device toggle */}
        {hasHtml && (
          <div className="flex items-center gap-1">
            <button
              onClick={() => setDeviceMode('desktop')}
              className={`flex items-center gap-1 text-[10px] px-2.5 py-1 rounded-md transition-premium ${
                deviceMode === 'desktop' ? 'bg-amber-500/10 text-amber-500 border border-amber-500/30' : 'text-gray-500 hover:text-gray-300 border border-transparent'
              }`}
            >
              <Desktop size={13} /> {t('reportDetail.device.desktop')}
            </button>
            <button
              onClick={() => setDeviceMode('mobile')}
              className={`flex items-center gap-1 text-[10px] px-2.5 py-1 rounded-md transition-premium ${
                deviceMode === 'mobile' ? 'bg-amber-500/10 text-amber-500 border border-amber-500/30' : 'text-gray-500 hover:text-gray-300 border border-transparent'
              }`}
            >
              <DeviceMobile size={13} /> {t('reportDetail.device.mobile')}
            </button>
          </div>
        )}
        <div className="flex items-center gap-1.5 flex-1 justify-end">
          {/* Manual refresh */}
          <button
            onClick={() => { if (iframeRef.current && id) iframeRef.current.src = withToken(`/api/reports/${id}/html?t=${Date.now()}`); }}
            className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 px-2 py-1.5 rounded-md transition-premium"
            title={t('common.refresh')}
            aria-label={t('common.refresh')}
          >
            <ArrowClockwise size={11} />
          </button>
          <button onClick={() => setShowDsModal(true)} className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium">
            <Database size={11} /> {t('reportDetail.dataSources')} ({datasources.length})
          </button>
          {/* Report-level global filters */}
          <button
            onClick={() => setShowFilters(true)}
            className={`flex items-center gap-1 text-[10px] border px-2.5 py-1.5 rounded-md transition-premium ${
              (report.report_filters?.length ?? 0) > 0
                ? 'text-amber-500/90 border-amber-500/30 hover:text-amber-400'
                : 'text-gray-400 border-obsidian-700 hover:text-gray-200'
            }`}
          >
            <Funnel size={11} weight={(report.report_filters?.length ?? 0) > 0 ? 'fill' : 'regular'} />
            {t('reportDetail.globalFilters.button')}
            {(report.report_filters?.length ?? 0) > 0 && ` (${report.report_filters!.length})`}
          </button>
          {/* Refresh interval */}
          <RefreshIntervalPicker value={refreshInterval} onChange={(v) => { setRefreshInterval(v); if (report) reportsApi.updateRefreshInterval(report.id, v).catch(() => {}); }} t={t} />
          {/* Debug SQL toggle */}
          <button
            onClick={() => setShowDebug((v) => !v)}
            className={`flex items-center gap-1 text-[10px] border px-2 py-1.5 rounded-md transition-premium ${
              showDebug
                ? 'text-amber-500/90 border-amber-500/30 hover:text-amber-400'
                : 'text-gray-400 border-obsidian-700 hover:text-gray-200'
            }`}
            title={t('reportDetail.debug.button')}
            aria-label={t('reportDetail.debug.button')}
          >
            <Bug size={11} weight={showDebug ? 'fill' : 'regular'} /> {t('reportDetail.debug.button')}
          </button>
          <button onClick={handlePublish} className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium">
            <Upload size={11} /> {report.status === 'published' ? t('reportDetail.unpublish') : t('reportDetail.publish')}
          </button>
          {/* Rollback to published version */}
          {report.published_html && report.html_content !== report.published_html && (
            <button onClick={handleRollback} className="flex items-center gap-1 text-[10px] text-amber-500/80 hover:text-amber-400 border border-amber-500/20 px-2.5 py-1.5 rounded-md transition-premium" title={t('reportDetail.rollbackHint')}>
              <ArrowCounterClockwise size={11} /> {t('reportDetail.rollback')}
            </button>
          )}

          <button onClick={handleShare} className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium">
            <ShareNetwork size={11} /> {t('reportDetail.share')}
          </button>
          <button
            onClick={toggleSummary}
            className={`flex items-center gap-1 text-[10px] border px-2.5 py-1.5 rounded-md transition-premium ${showSummary ? 'text-amber-500 border-amber-500/30 bg-amber-500/5' : 'text-gray-400 hover:text-gray-200 border-obsidian-700'}`}
            title={t('reportDetail.summary.title')}
          >
            <Lightbulb size={11} /> {t('reportDetail.summary.button')}
          </button>
          <button
            onClick={async () => {
              setShowVersions(!showVersions);
              if (!showVersions && report) {
                const res = await fetch(`/api/reports/${report.id}/versions`, { headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` } });
                if (res.ok) setVersions(await res.json());
              }
            }}
            className={`flex items-center gap-1 text-[10px] border px-2.5 py-1.5 rounded-md transition-premium ${showVersions ? 'text-amber-500 border-amber-500/30 bg-amber-500/5' : 'text-gray-400 hover:text-gray-200 border-obsidian-700'}`}
          >
            <ClockCounterClockwise size={11} /> {t('reportDetail.versions')}
          </button>
        </div>
      </div>

      {error && <div className="px-5 pt-2"><ErrorBanner message={error} onDismiss={() => setError(null)} /></div>}

      {/* ── HTML Preview (iframe) ── */}
      <div className="flex-1 overflow-hidden bg-obsidian-950 relative flex flex-row">

        {/* AI Summary + Q&A Drawer (right side) */}
        {showSummary && (
          <div className="w-96 bg-obsidian-900 border-l border-obsidian-700 flex flex-col flex-shrink-0 order-2">
            <div className="flex items-center justify-between px-3 py-2 border-b border-obsidian-700 flex-shrink-0">
              <span className="text-[11px] text-gray-300 font-medium flex items-center gap-1.5">
                <Lightbulb size={13} className="text-amber-500" /> {t('reportDetail.summary.title')}
              </span>
              <button onClick={() => setShowSummary(false)} aria-label={t('common.close')} className="text-gray-500 hover:text-gray-300"><X size={12} /></button>
            </div>

            {/* Scrollable: summary block + Q&A conversation */}
            <div className="flex-1 overflow-y-auto scrollbar-thin p-3">
              {summaryLoading ? (
                <div className="flex flex-col items-center justify-center py-10 gap-3">
                  <div className="relative w-7 h-7">
                    <div className="absolute inset-0 border-2 border-amber-500/20 rounded-full" />
                    <div className="absolute inset-0 border-2 border-transparent border-t-amber-500 rounded-full animate-spin" />
                  </div>
                  <span className="text-[10px] text-gray-500">{t('reportDetail.summary.generating')}</span>
                </div>
              ) : summary ? (
                <div className="space-y-3">
                  <p className="text-xs text-gray-100 font-medium leading-relaxed bg-amber-500/5 border border-amber-500/15 rounded-lg p-2.5">
                    {summary.headline}
                  </p>
                  <SummarySection title={t('reportDetail.summary.highlights')} items={summary.highlights} color="text-data-green" />
                  <SummarySection title={t('reportDetail.summary.trends')} items={summary.trends} color="text-amber-500" />
                  <SummarySection title={t('reportDetail.summary.anomalies')} items={summary.anomalies} color="text-red-400" />
                  <SummarySection title={t('reportDetail.summary.recommendations')} items={summary.recommendations} color="text-blue-400" />
                  {summaryUpdatedAt && (
                    <p className="text-[8px] text-gray-600 pt-1">
                      {t('reportDetail.summary.updatedAt')}: {new Date(summaryUpdatedAt).toLocaleString()}
                    </p>
                  )}
                  <button
                    onClick={handleGenerateSummary}
                    className="w-full flex items-center justify-center gap-1.5 text-[10px] text-gray-300 border border-obsidian-700 hover:border-amber-500/30 hover:text-amber-500 py-1.5 rounded-md transition-premium"
                  >
                    <ArrowClockwise size={11} /> {t('reportDetail.summary.regenerate')}
                  </button>
                </div>
              ) : (
                <div className="flex flex-col items-center justify-center py-8 px-2 text-center gap-3">
                  <Lightbulb size={28} className="text-amber-500/30" />
                  <p className="text-[10px] text-gray-500">{t('reportDetail.summary.emptyHint')}</p>
                  <button
                    onClick={handleGenerateSummary}
                    disabled={datasources.length === 0}
                    className="flex items-center gap-1.5 text-[10px] text-[#08080c] bg-amber-500 hover:bg-amber-400 font-semibold px-3 py-1.5 rounded-md transition-premium disabled:opacity-40"
                  >
                    <Sparkle size={11} weight="fill" /> {t('reportDetail.summary.generate')}
                  </button>
                </div>
              )}

              {/* Q&A conversation */}
              <div className="mt-4 pt-3 border-t border-obsidian-700/60">
                <span className="text-[9px] font-semibold uppercase tracking-wide text-gray-500 flex items-center gap-1">
                  <ChatCircle size={11} className="text-amber-500" /> {t('reportDetail.summary.qaTitle')}
                </span>
                {qaMessages.length === 0 && !qaLoading && (
                  <p className="text-[10px] text-gray-600 mt-2 leading-relaxed">{t('reportDetail.summary.qaHint')}</p>
                )}
                <div className="mt-2 space-y-2">
                  {qaMessages.map((m, idx) => (
                    <div key={idx} className={`flex ${m.role === 'user' ? 'justify-end' : 'justify-start'}`}>
                      <div className={`max-w-[85%] rounded-lg px-2.5 py-1.5 text-[11px] leading-relaxed whitespace-pre-wrap ${
                        m.role === 'user'
                          ? 'bg-amber-500/15 text-amber-100 border border-amber-500/20'
                          : 'bg-obsidian-800 text-gray-300 border border-obsidian-700'
                      }`}>
                        {m.content}
                      </div>
                    </div>
                  ))}
                  {qaLoading && (
                    <div className="flex justify-start">
                      <div className="bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 flex items-center gap-1">
                        <span className="w-1 h-1 bg-amber-500 rounded-full animate-pulse" />
                        <span className="w-1 h-1 bg-amber-500 rounded-full animate-pulse" style={{ animationDelay: '150ms' }} />
                        <span className="w-1 h-1 bg-amber-500 rounded-full animate-pulse" style={{ animationDelay: '300ms' }} />
                      </div>
                    </div>
                  )}
                  <div ref={qaEndRef} />
                </div>
              </div>
            </div>

            {/* Q&A input (fixed at bottom) */}
            <div className="border-t border-obsidian-700 p-2 flex-shrink-0">
              <div className="relative">
                <input
                  value={qaInput}
                  onChange={(e) => setQaInput(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleAsk(); } }}
                  placeholder={t('reportDetail.summary.qaPlaceholder')}
                  disabled={qaLoading || datasources.length === 0}
                  className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg pl-3 pr-9 py-2 text-[11px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium disabled:opacity-50"
                />
                <button
                  onClick={handleAsk}
                  disabled={qaLoading || !qaInput.trim()}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-500 hover:text-amber-500 disabled:text-gray-700 transition-premium"
                  aria-label={t('reportDetail.summary.qaSend')}
                >
                  <PaperPlaneRight size={13} weight="fill" />
                </button>
              </div>
            </div>
          </div>
        )}

        {/* Version History Drawer (right side) */}
        {showVersions && (
          <div className="w-48 bg-obsidian-900 border-l border-obsidian-700 flex flex-col flex-shrink-0 order-2">
            <div className="flex items-center justify-between px-3 py-2 border-b border-obsidian-700">
              <span className="text-[10px] text-gray-300 font-medium">{t('reportDetail.versions')}</span>
              <button onClick={() => { setShowVersions(false); setCompareVersionId(null); }} aria-label={t('common.close')} className="text-gray-500 hover:text-gray-300"><X size={12} /></button>
            </div>
            <div className="flex-1 overflow-y-auto scrollbar-thin p-2 space-y-1.5">
              {versions.length === 0 && (
                <p className="text-[9px] text-gray-600 italic text-center py-4">{t('reportDetail.noVersions')}</p>
              )}
              {versions.map((v) => (
                <div
                  key={v.id}
                  onClick={() => {
                    if (compareVersionId === v.id) {
                      setCompareVersionId(null);
                      // Revert to report's current style
                      if (report) setSelectedStyleKey(report.style_key || null);
                    } else {
                      setCompareVersionId(v.id);
                      // Show this version's style in the dock
                      setSelectedStyleKey(v.style_key || null);
                    }
                  }}
                  className={`rounded-lg p-2 border cursor-pointer transition-premium ${
                    compareVersionId === v.id
                      ? 'border-amber-500/50 bg-amber-500/10'
                      : 'border-obsidian-700 hover:border-obsidian-600 hover:bg-obsidian-800'
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <span className={`text-[11px] font-mono font-semibold ${compareVersionId === v.id ? 'text-amber-500' : 'text-gray-300'}`}>
                      v{v.version}
                    </span>
                    <div className="flex items-center gap-1">
                      <button
                        onClick={async (e) => {
                          e.stopPropagation();
                          if (!report) return;
                          await fetch(`/api/reports/${report.id}/versions/${v.id}/restore`, {
                            method: 'POST',
                            headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
                          });
                          setCompareVersionId(null);
                          const fresh = await reportsApi.get(report.id);
                          setReport(fresh);
                          refreshVersion(report.id);
                          if (iframeRef.current) iframeRef.current.src = withToken(`/api/reports/${report.id}/html?t=${Date.now()}`);
                          toast.success(t('reportDetail.versionRestored'));
                        }}
                        className="text-gray-600 hover:text-amber-500 transition-premium"
                        title={t('reportDetail.restoreVersion')}
                        aria-label={t('reportDetail.restoreVersion')}
                      >
                        <ArrowCounterClockwise size={12} />
                      </button>
                      <button
                        onClick={async (e) => {
                          e.stopPropagation();
                          if (!report) return;
                          setDeleteVersionTarget({ id: v.id, version: v.version });
                        }}
                        className="text-gray-600 hover:text-red-400 transition-premium"
                        title={t('reportDetail.deleteVersion')}
                        aria-label={t('reportDetail.deleteVersion')}
                      >
                        <Trash size={12} />
                      </button>
                    </div>
                  </div>
                  <span className="text-[8px] text-gray-500 block mt-0.5">
                    {v.created_at ? new Date(v.created_at).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : ''}
                  </span>
                  {v.prompt && <p className="text-[8px] text-gray-500 mt-0.5 truncate">{v.prompt}</p>}
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Main preview area */}
        <div className="flex-1 flex flex-col overflow-hidden order-1">
          <div className={`flex-1 overflow-auto relative ${deviceMode === 'mobile' && hasHtml ? 'flex items-start justify-center py-4' : ''}`}>

        {/* AI Loading Overlay */}
        {aiLoading && (
          <div className="absolute inset-0 z-10 bg-obsidian-950/90 flex flex-col items-center justify-center backdrop-blur-sm">
            {/* Animated dashboard skeleton */}
            <div className="w-80 mb-8">
              <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-4 overflow-hidden">
                {/* Fake KPI row */}
                <div className="flex gap-2 mb-3">
                  {[1,2,3].map(i => (
                    <div key={i} className="flex-1 h-10 rounded-lg bg-obsidian-800 shimmer-surface border border-obsidian-700" style={{ animationDelay: `${i * 200}ms` }} />
                  ))}
                </div>
                {/* Fake chart area */}
                <div className="h-24 rounded-lg bg-obsidian-800 shimmer-surface border border-obsidian-700 mb-3" style={{ animationDelay: '600ms' }} />
                {/* Fake bottom row */}
                <div className="flex gap-2">
                  <div className="flex-1 h-16 rounded-lg bg-obsidian-800 shimmer-surface border border-obsidian-700" style={{ animationDelay: '800ms' }} />
                  <div className="flex-1 h-16 rounded-lg bg-obsidian-800 shimmer-surface border border-obsidian-700" style={{ animationDelay: '1000ms' }} />
                </div>
              </div>
            </div>
            {/* Loading text with animation */}
            <div className="flex items-center gap-3 mb-3">
              <div className="relative w-8 h-8">
                <div className="absolute inset-0 border-2 border-amber-500/20 rounded-full" />
                <div className="absolute inset-0 border-2 border-transparent border-t-amber-500 rounded-full animate-spin" />
                <Sparkle size={14} className="absolute inset-0 m-auto text-amber-500" />
              </div>
              <span className="text-sm text-gray-300 font-medium">{t('reportDetail.aiLoading.title')}</span>
            </div>
            <LoadingMessages t={t} />
            <button
              onClick={handleAIStop}
              className="mt-4 flex items-center gap-1.5 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 hover:border-obsidian-600 px-3 py-1.5 rounded-lg transition-premium"
            >
              <Stop size={11} /> {t('reportDetail.runInBackground')}
            </button>
            <p className="text-[9px] text-gray-600 mt-2">{t('reportDetail.backgroundHint')}</p>
          </div>
        )}

        {hasHtml || compareVersionId ? (
          <div className={deviceMode === 'mobile' && !compareVersionId ? 'w-[375px] h-[667px] border border-obsidian-700 rounded-2xl overflow-hidden shadow-2xl bg-obsidian-950 flex-shrink-0' : 'w-full h-full'}>
            {compareVersionId && report && (
              <div className="absolute top-2 left-2 z-10 text-[9px] bg-obsidian-900/90 border border-amber-500/30 rounded px-2 py-0.5 text-amber-500">
                v{versions.find(v => v.id === compareVersionId)?.version} — {t('reportDetail.oldVersion')}
              </div>
            )}
            <iframe
              ref={iframeRef}
              key={compareVersionId ? `ver-${compareVersionId}` : report?.updated_at}
              src={compareVersionId && report ? withToken(`/api/reports/${report.id}/versions/${compareVersionId}/html`) : iframeSrc}
              className="w-full h-full border-0"
              sandbox="allow-scripts allow-same-origin"
              title={report?.title}
            />
          </div>
        ) : (
          <div className="h-full flex flex-col items-center justify-center px-6">
            <Sparkle size={32} className="text-amber-500/30 mb-4" />
            <p className="text-sm text-gray-300 font-medium mb-2">{t('reportDetail.htmlEmpty.title')}</p>
            <p className="text-[11px] text-gray-500 max-w-md text-center mb-4">{t('reportDetail.htmlEmpty.description')}</p>
            {datasources.length === 0 && (
              <button onClick={() => setShowDsModal(true)} className="flex items-center gap-1.5 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-200 text-xs px-4 py-2 rounded-lg transition-premium mb-3">
                <Plus size={14} /> {t('reportDetail.htmlEmpty.addData')}
              </button>
            )}
            <p className="text-[10px] text-gray-600">{t('reportDetail.htmlEmpty.hint')}</p>
          </div>
        )}
          </div>{/* close inner scroll */}
        </div>{/* close order-1 main preview */}
      </div>

      {/* ── Debug SQL panel (below the report, only when toggled on) ── */}
      {showDebug && report && (
        <ReportDebugPanel reportId={report.id} onClose={() => setShowDebug(false)} />
      )}

      {/* ── AI Chat Bar (bottom) with Dock-style selector ── */}
      <div className="flex-shrink-0 border-t border-obsidian-700 px-5 py-3 bg-obsidian-900 relative">
        <div className="flex items-center gap-2 max-w-4xl mx-auto">
          {/* Left: Dock style selector */}
          {!aiLoading && (
            <StyleDock
              selectedKey={selectedStyleKey}
              onSelect={(key, prompt) => {
                setSelectedStyleKey(key);
                if (report) reportsApi.updateStyle(report.id, key).catch(() => {});
                handleAISendPrompt(prompt);
              }}
            />
          )}

          {/* Input with selected style indicator */}
          <div className="relative flex-1 min-w-0">
            {selectedStyleKey ? (
              <button
                onClick={() => { setSelectedStyleKey(null); if (report) reportsApi.updateStyle(report.id, null).catch(() => {}); }}
                className="absolute left-2.5 top-1/2 -translate-y-1/2 text-sm leading-none z-10"
                title={t('reportDetail.clearStyle')}
              >
                {DOCK_STYLES.find(s => s.key === selectedStyleKey)?.emoji || '✨'}
              </button>
            ) : (
              <Sparkle size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-amber-500" />
            )}
            <input
              value={aiInput}
              onChange={(e) => setAiInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); handleAISend(); } }}
              placeholder={hasHtml ? t('reportDetail.aiRefineHint') : t('reportDetail.aiGenerateHint')}
              disabled={aiLoading}
              className="w-full bg-obsidian-800 border border-obsidian-700 rounded-xl pl-9 pr-10 py-2.5 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium disabled:opacity-50"
            />
            <button
              onClick={aiLoading ? handleAIStop : handleAISend}
              disabled={!aiLoading && !aiInput.trim()}
              className={`absolute right-3 top-1/2 -translate-y-1/2 transition-premium ${aiLoading ? 'text-red-400 hover:text-red-300' : 'text-gray-500 hover:text-amber-500 disabled:text-gray-700'}`}
              title={aiLoading ? t('reportDetail.stopGenerate') : undefined}
            >
              {aiLoading ? <Stop size={16} weight="fill" /> : <PaperPlaneRight size={14} weight="fill" />}
            </button>
          </div>

          {/* Right: one-click generate */}
          {!aiLoading && (
            <button
              onClick={handleQuickGenerate}
              disabled={aiLoading || datasources.length === 0}
              className="flex items-center gap-1.5 text-[10px] text-[#08080c] bg-amber-500 hover:bg-amber-400 font-semibold px-3 py-2 rounded-lg whitespace-nowrap transition-premium disabled:opacity-40 active:translate-y-[1px] flex-shrink-0"
            >
              <Sparkle size={11} weight="fill" />
              {t('reportDetail.quickActions.generate')}
            </button>
          )}

          {/* Right: saved themes */}
          <div className="relative flex-shrink-0">
            <button
              onClick={toggleThemes}
              title={t('reportDetail.themes.title')}
              className={`flex items-center justify-center w-9 h-9 rounded-lg border transition-premium ${
                showThemes
                  ? 'text-amber-500 border-amber-500/40 bg-amber-500/10'
                  : 'text-gray-400 border-obsidian-700 hover:text-amber-500 hover:border-amber-500/30'
              }`}
            >
              <Palette size={15} />
            </button>
            {showThemes && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setShowThemes(false)} />
                <div className="absolute bottom-full right-0 mb-2 z-40 w-80 bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl overflow-hidden">
                  <div className="px-3 py-2 border-b border-obsidian-700 flex items-center gap-2">
                    <Palette size={13} className="text-amber-500" />
                    <span className="text-[11px] text-gray-300 font-medium">{t('reportDetail.themes.title')}</span>
                    <button
                      onClick={() => { setShowThemes(false); setShowSaveTheme(true); }}
                      disabled={!hasHtml}
                      className="ml-auto flex items-center gap-1 text-[9px] text-amber-500 hover:text-amber-400 border border-amber-500/30 px-1.5 py-0.5 rounded disabled:opacity-40"
                      title={t('reportDetail.themes.saveCurrentHint')}
                    >
                      <Plus size={10} /> {t('reportDetail.themes.saveCurrent')}
                    </button>
                  </div>
                  <div className="max-h-72 overflow-y-auto scrollbar-thin py-1">
                    {themes.length === 0 ? (
                      <p className="text-[10px] text-gray-600 text-center py-5 italic">{t('reportDetail.themes.empty')}</p>
                    ) : (
                      themes.map((th) => (
                        <div
                          key={th.id}
                          className="w-full flex items-start gap-2 px-3 py-2 hover:bg-obsidian-800 transition-premium group border-b border-obsidian-800/60 last:border-0"
                        >
                          <button
                            onClick={() => handleUseTheme(th)}
                            className="flex items-start gap-2 flex-1 text-left min-w-0"
                            title={t('reportDetail.themes.useHint')}
                          >
                            <span className="text-base leading-none mt-0.5">{th.emoji}</span>
                            <span className="min-w-0">
                              <span className="block text-[11px] text-gray-200 group-hover:text-amber-500 font-medium truncate">{th.name}</span>
                              {th.description && <span className="block text-[9px] text-gray-500 truncate">{th.description}</span>}
                            </span>
                          </button>
                          <button
                            onClick={() => handleDeleteTheme(th.id)}
                            className="text-gray-700 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-premium flex-shrink-0 mt-0.5"
                            title={t('common.delete')}
                            aria-label={t('common.delete')}
                          >
                            <Trash size={11} />
                          </button>
                        </div>
                      ))
                    )}
                  </div>
                </div>
              </>
            )}
          </div>

          {/* Right: ask history */}
          <div className="relative flex-shrink-0">
            <button
              onClick={toggleHistory}
              title={t('reportDetail.askHistory')}
              className={`flex items-center justify-center w-9 h-9 rounded-lg border transition-premium ${
                showHistory
                  ? 'text-amber-500 border-amber-500/40 bg-amber-500/10'
                  : 'text-gray-400 border-obsidian-700 hover:text-amber-500 hover:border-amber-500/30'
              }`}
            >
              <ClockCounterClockwise size={15} />
            </button>
            {showHistory && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setShowHistory(false)} />
                <div className="absolute bottom-full right-0 mb-2 z-40 w-80 bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl overflow-hidden">
                  <div className="px-3 py-2 border-b border-obsidian-700 flex items-center gap-2">
                    <ClockCounterClockwise size={13} className="text-amber-500" />
                    <span className="text-[11px] text-gray-300 font-medium">{t('reportDetail.askHistory')}</span>
                    <span className="text-[10px] text-gray-600 ml-auto">{history.filter(h => h.prompt).length}</span>
                  </div>
                  <div className="max-h-72 overflow-y-auto scrollbar-thin py-1">
                    {history.filter(h => h.prompt).length === 0 ? (
                      <p className="text-[10px] text-gray-600 text-center py-5 italic">{t('reportDetail.noAskHistory')}</p>
                    ) : (
                      history.filter(h => h.prompt).map((h) => (
                        <button
                          key={h.id}
                          onClick={() => { setAiInput(h.prompt || ''); setShowHistory(false); }}
                          className="w-full text-left px-3 py-2 hover:bg-obsidian-800 transition-premium group border-b border-obsidian-800/60 last:border-0"
                          title={t('reportDetail.reusePrompt')}
                        >
                          <div className="flex items-center gap-1.5 mb-0.5">
                            <span className="text-[8px] text-amber-500/70 font-mono">v{h.version}</span>
                            <span className="text-[8px] text-gray-600">
                              {h.created_at ? new Date(h.created_at).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) : ''}
                            </span>
                          </div>
                          <p className="text-[11px] text-gray-300 group-hover:text-gray-100 line-clamp-2">{h.prompt}</p>
                        </button>
                      ))
                    )}
                  </div>
                </div>
              </>
            )}
          </div>
        </div>
        {aiLoading && <p className="text-center text-[10px] text-amber-500/60 mt-1.5">{t('reportDetail.aiGenerating')}</p>}
      </div>

      {/* ── Report-level global filters ── */}
      {showFilters && report && (
        <ReportFilters
          report={report}
          datasources={datasources}
          onClose={() => setShowFilters(false)}
          onApplied={(updated) => {
            setReport(updated);
            setShowFilters(false);
            if (iframeRef.current && id) {
              iframeRef.current.src = withToken(`/api/reports/${id}/html?t=${Date.now()}`);
            }
          }}
        />
      )}

      {/* ── Datasource Modal ── */}
      {showDsModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setShowDsModal(false)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-2xl w-[420px] max-h-[70vh] overflow-hidden shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-obsidian-700">
              <h2 className="text-sm font-semibold text-gray-200">{t('reportDetail.dataSources')}</h2>
              <button onClick={() => setShowDsModal(false)} className="text-gray-500 hover:text-gray-300"><X size={16} /></button>
            </div>
            <div className="p-4 overflow-y-auto max-h-[50vh] scrollbar-thin space-y-3">
              {/* Current datasources */}
              {datasources.length > 0 && (
                <div>
                  <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium">{t('reportDetail.currentData')}</span>
                  <div className="mt-1.5 space-y-1">
                    {datasources.map((ds) => (
                      <div key={ds.id} className="px-3 py-2 rounded-lg bg-obsidian-800 border border-obsidian-700 group">
                        <div className="flex items-center gap-2">
                          <Database size={12} className="text-data-green" />
                          <span className="text-xs text-gray-300 flex-1 truncate">{ds.name}</span>
                          <span className="text-[9px] text-gray-600">{ds.row_count ?? 0} rows</span>
                          <button
                            onClick={async () => {
                              await reportsApi.removeDatasource(report!.id, ds.id).catch(() => {});
                              setDatasources((prev) => prev.filter((d) => d.id !== ds.id));
                            }}
                            className="text-gray-700 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-premium flex-shrink-0"
                          >
                            <X size={12} />
                          </button>
                        </div>
                        <DatasourceFilters
                          reportId={report!.id}
                          ds={ds}
                          onUpdated={(updated) =>
                            setDatasources((prev) => prev.map((d) => (d.id === updated.id ? updated : d)))
                          }
                        />
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {/* Add by group */}
              {metricGroups.length > 0 && (
                <div>
                  <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium">{t('reportDetail.addByGroup')}</span>
                  <div className="mt-1.5 space-y-1">
                    {metricGroups.map((g) => {
                      const gm = metrics.filter((m) => m.group_id === g.id && !datasources.some((d) => d.metric_id === m.id));
                      if (gm.length === 0) return null;
                      return (
                        <div key={g.id} onClick={() => handleAddGroup(g.id)} className="flex items-center gap-2 px-3 py-2 rounded-lg bg-amber-500/5 hover:bg-amber-500/10 border border-amber-500/20 cursor-pointer transition-premium">
                          <Star size={12} className="text-amber-500" weight="fill" />
                          <span className="text-xs text-amber-500/90 font-medium flex-1 truncate">{g.name}</span>
                          <span className="text-[9px] text-amber-500/60">{gm.length} {t('reportDetail.addByGroupCount')}</span>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}
              {/* Individual metrics */}
              <div>
                <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium">{t('reportDetail.addFromMetrics')}</span>
                <div className="mt-1.5 space-y-1">
                  {metrics.filter((m) => !datasources.some((d) => d.metric_id === m.id)).map((m) => (
                    <div key={m.id} onClick={() => { handleAddMetric(m); }} className="flex items-center gap-2 px-3 py-2 rounded-lg hover:bg-obsidian-800 border border-obsidian-700/50 cursor-pointer transition-premium">
                      <Star size={11} className="text-amber-500/60" weight="fill" />
                      <span className="text-xs text-gray-400 flex-1 truncate">{m.name}</span>
                      <Plus size={11} className="text-gray-600" />
                    </div>
                  ))}
                  {metrics.length === 0 && <p className="text-[10px] text-gray-600 italic">{t('metrics.empty.sidebar')}</p>}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Save Theme Modal */}
      {showSaveTheme && report && (
        <SaveThemeModal
          reportId={report.id}
          t={t}
          onClose={() => setShowSaveTheme(false)}
          onSaved={(th) => { setThemes((prev) => [th, ...prev]); setShowSaveTheme(false); toast.success(t('reportDetail.themes.saved')); }}
        />
      )}

      {/* Delete Version Confirmation Modal */}
      {deleteVersionTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setDeleteVersionTarget(null)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-sm font-semibold text-gray-100 mb-2">{t('reportDetail.deleteVersionConfirm.title')}</h3>
            <p className="text-xs text-gray-400 mb-4">{t('reportDetail.deleteVersionConfirm.message', { version: deleteVersionTarget.version })}</p>
            <div className="flex items-center justify-end gap-2">
              <button
                onClick={() => setDeleteVersionTarget(null)}
                className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={async () => {
                  if (!report) return;
                  const res = await fetch(`/api/reports/${report.id}/versions/${deleteVersionTarget.id}`, {
                    method: 'DELETE',
                    headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
                  });
                  if (!res.ok) {
                    setDeleteVersionTarget(null);
                    toast.error(t('errors.deleteFailed'));
                    return;
                  }
                  if (compareVersionId === deleteVersionTarget.id) setCompareVersionId(null);
                  setVersions(versions.filter(ver => ver.id !== deleteVersionTarget.id));
                  setDeleteVersionTarget(null);
                  toast.success(t('reportDetail.versionDeleted'));
                }}
                className="text-xs text-white bg-red-600 hover:bg-red-500 px-3 py-1.5 rounded-md transition-premium"
              >
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
// ── AI summary section (bulleted list; hidden when empty) ──
function SummarySection({ title, items, color }: { title: string; items: string[]; color: string }) {
  if (!items || items.length === 0) return null;
  return (
    <div>
      <span className={`text-[9px] font-semibold uppercase tracking-wide ${color}`}>{title}</span>
      <ul className="mt-1 space-y-1">
        {items.map((it, i) => (
          <li key={i} className="text-[11px] text-gray-300 leading-relaxed flex gap-1.5">
            <span className={`${color} flex-shrink-0`}>•</span>
            <span>{it}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function LoadingMessages({ t }: { t: (k: string) => string }) {
  const [msgIndex, setMsgIndex] = useState(0);
  const messages = [
    t('reportDetail.aiLoading.msg1'),
    t('reportDetail.aiLoading.msg2'),
    t('reportDetail.aiLoading.msg3'),
    t('reportDetail.aiLoading.msg4'),
    t('reportDetail.aiLoading.msg5'),
    t('reportDetail.aiLoading.msg6'),
  ];

  useEffect(() => {
    const timer = setInterval(() => {
      setMsgIndex((prev) => (prev + 1) % messages.length);
    }, 3000);
    return () => clearInterval(timer);
  }, [messages.length]);

  return (
    <p className="text-[11px] text-gray-500 animate-pulse transition-all duration-500">
      {messages[msgIndex]}
    </p>
  );
}

// ── Custom Refresh Interval Picker (dark themed dropdown) ──
function RefreshIntervalPicker({ value, onChange, t }: { value: number; onChange: (v: number) => void; t: (k: string) => string }) {
  const [open, setOpen] = useState(false);
  const options = [
    { value: 0, label: t('reportDetail.refresh.off') },
    { value: 1, label: `1 ${t('reportDetail.refresh.min')}` },
    { value: 5, label: `5 ${t('reportDetail.refresh.min')}` },
    { value: 15, label: `15 ${t('reportDetail.refresh.min')}` },
    { value: 30, label: `30 ${t('reportDetail.refresh.min')}` },
    { value: 60, label: `1 ${t('reportDetail.refresh.hour')}` },
  ];
  const current = options.find((o) => o.value === value) || options[5];

  return (
    <div className="relative">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 text-[10px] text-gray-400 hover:text-gray-200 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium"
      >
        <Clock size={11} />
        <span>{current.label}</span>
        <CaretDown size={9} className={`transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
          <div className="absolute right-0 top-full mt-1 z-50 bg-obsidian-900 border border-obsidian-700 rounded-lg shadow-2xl py-1 min-w-[120px]">
            {options.map((opt) => (
              <button
                key={opt.value}
                onClick={() => { onChange(opt.value); setOpen(false); }}
                className={`w-full text-left px-3 py-1.5 text-[10px] transition-premium ${
                  opt.value === value
                    ? 'text-amber-500 bg-amber-500/10'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

// ── macOS Dock-style Style Selector ──
const DOCK_STYLES = [
  // ── Dark & Tech ──
  { key: 'obsidian', emoji: '🖤', label: '黑曜石', prompt: '完全重新设计整个页面视觉风格为【黑曜石 · 高端克制】：分层背景 页面#0b0b11／卡片#14141d／悬浮#1b1b26，绝不用纯黑；主色金#d4a853，文字#e8e8ec/次级#9aa0ab；字体Inter，标题weight600 letter-spacing-0.02em，数字一律tabular-nums；KPI数值40px金色+下方11px大写灰标签letter-spacing0.08em，仅在数据支持时显示▲▼涨跌色(绿#34d399/红#f87171)；卡片1px描边rgba(255,255,255,0.07) 圆角15px padding24px，hover时上浮1px+描边转金色，transition200ms cubic-bezier(0.4,0,0.2,1)；3列grid gap20px居中max-width1360px；图表透明底、网格线极淡虚线、坐标轴隐藏、配色金#d4a853+青#22d3ee+紫#a78bfa，环形图替代饼图；气质像奢侈品牌的高管仪表盘' },
  { key: 'neon', emoji: '💚', label: '赛博霓虹', prompt: '完全重新设计整个页面视觉风格为【赛博霓虹】：背景#05060a，唯一强调色霓虹绿#00ff88；字体JetBrains Mono，数字tabular-nums；KPI数值用text-shadow:0 0 24px #00ff88aa做克制发光（不要糊成一团），标签小写灰绿；卡片纯方角border-radius0 1px实线#00ff8833 背景#0a0c10，hover时描边变亮+box-shadow:0 0 16px #00ff8822；主+右窄列8:4布局；叠加极淡扫描线(repeating-linear-gradient每4px一条1px半透明绿)；图表只用绿色明度梯度、线条0.5px、网格隐藏；气质冷峻精密的终端面板' },
  { key: 'midnight', emoji: '🌌', label: '深空', prompt: '完全重新设计整个页面视觉风格为【深空】：背景从#0a0a1a到#0e0e26柔和径向渐变；主色薰衣草紫#a78bfa+星光白#f1f5f9，文字次级#8b8fa3；字体Inter标题weight500正文weight300，数字tabular-nums；KPI数值44px白色+紫色小标签；卡片background:rgba(18,18,40,0.6) backdrop-filter:blur(14px) 1px描边rgba(167,139,250,0.16) 圆角20px；宽松对称2列 gap24px；图表配色紫#a78bfa+蓝#60a5fa+粉#f472b6，面积图带极淡竖向渐变、网格隐藏；气质静谧高级的太空站控制台' },

  // ── Business & Professional ──
  { key: 'corporate', emoji: '🔵', label: '商务蓝', prompt: '完全重新设计整个页面视觉风格为【商务蓝】：背景#0f172a深海军蓝/卡片#172033；主色#3b82f6，文字白#f8fafc/次级#94a3b8；字体Inter正文weight400标题weight600，数字tabular-nums；首排KPI用一张蓝色渐变主卡(linear-gradient135deg #1e40af→#3b82f6)白字突出核心指标，其余卡片#172033 1px描边#27374d 圆角12px；3列grid gap16px居中；表格斑马纹(#172033/#0f172a交替)、表头深蓝、行hover高亮；图表蓝色明度梯度#3b82f6/#60a5fa/#93c5fd、网格极淡；气质沉稳可信的企业级BI' },
  { key: 'executive', emoji: '👔', label: '总裁灰', prompt: '完全重新设计整个页面视觉风格为【总裁灰 · 极简】：背景#161619锌灰/卡片#1f1f23；纯灰度+唯一强调翡翠绿#10b981；字体Inter，标题weight500、KPI数值56px weight200超细、tabular-nums，绿色只用于正向▲；卡片无边框 圆角10px 极淡阴影0 1px 3px rgba(0,0,0,0.4)；4列KPI行+2列图表行，留白极大padding32px gap24px，用1px hairline分隔区块；图表只用灰#71717a+绿#10b981两色、线条克制、无网格；气质CEO专属的留白美学仪表盘' },

  // ── Modern & Trendy ──
  { key: 'aurora', emoji: '🟣', label: '极光', prompt: '完全重新设计整个页面视觉风格为【极光】：背景#0b0a18，一个fixed层做极光径向渐变(紫#7c3aed左上+蓝#2563eb右下+粉#ec4899右上 各opacity0.10 blur后铺底)；字体Inter weight400，数字tabular-nums；卡片background:rgba(255,255,255,0.035) backdrop-filter:blur(20px) 1px描边rgba(255,255,255,0.08) 超大圆角22px；KPI数值用linear-gradient(135deg,#a78bfa,#60a5fa) background-clip:text渐变字48px；非对称布局(首行一张大卡span2+两张小卡)；图表紫粉蓝渐变、面积图柔和；气质梦幻通透的新潮看板' },
  { key: 'glassmorphism', emoji: '💎', label: '玻璃态', prompt: '完全重新设计整个页面视觉风格为【玻璃态】：背景#0d1117，铺2-3个大彩色模糊光斑(absolute div 420px 圆形 filter:blur(110px) opacity0.16 颜色#7c3aed/#0ea5e9/#f97316)；字体Inter weight300，数字tabular-nums；卡片background:rgba(255,255,255,0.045) backdrop-filter:blur(24px) 1px描边rgba(255,255,255,0.09) 圆角16px，hover提亮；KPI数值白色48px weight200；2列等宽gap16px；图表用半透明描边、极淡填充、网格隐藏；气质通透轻盈的高级玻璃质感' },
  { key: 'brutalist', emoji: '⬛', label: '粗野主义', prompt: '完全重新设计整个页面视觉风格为【粗野主义】：背景暖白纸色#f5f5f4，文字纯黑#0a0a0a；标题超粗黑体weight900 大写 letter-spacing0.04em，正文用mono，数字tabular-nums；KPI数值72px weight900 行高1；卡片无背景 3px实线黑边 纯方角 padding20px；刻意非对称布局(左窄30%右宽70%或交错)；唯一强调亮黄#facc15；图表只用黑+黄、粗线条、无阴影无渐变；气质先锋艺术展海报般的强冲击数据墙' },

  // ── Warm & Creative ──
  { key: 'sunset', emoji: '🌅', label: '日落暖橙', prompt: '完全重新设计整个页面视觉风格为【日落暖橙】：背景#17110f暗巧克力/卡片#211712；主色橙#f97316→玫红#e11d48渐变，文字#f5e9e2/次级#b59a8f；字体Inter weight500，数字tabular-nums；KPI数值用橙玫渐变background-clip:text 44px；卡片1px描边#3a221c 圆角14px，左侧4px渐变竖条点题，hover橙色微光；3列gap20px；图表暖色系#f97316/#ef4444/#eab308/#f472b6，面积图带渐变；气质热烈而不刺眼的活力看板' },
  { key: 'mint', emoji: '🌿', label: '薄荷清新', prompt: '完全重新设计整个页面视觉风格为【薄荷清新】：背景#f4fbf6极浅薄荷/卡片纯白；主色翠绿#059669，文字#14241d/次级#5b6b63；字体Inter正文weight400标题weight600，数字tabular-nums；KPI数值38px weight700绿色+灰标签；卡片1px描边#d5efe0 圆角14px 柔影0 2px 10px rgba(5,150,105,0.07)，hover微抬；3列等宽gap18px；表格绿色表头白字、行hover浅绿；图表绿色明度梯度#059669/#34d399/#6ee7b7、网格极淡；气质清爽专业的医疗/环保风仪表盘' },

  // ── Classic & Elegant ──
  { key: 'ivory', emoji: '📜', label: '象牙学术', prompt: '完全重新设计整个页面视觉风格为【象牙学术】：背景象牙白#faf9f7，文字暖黑#292524/次级#78716c；标题用衬线体(Georgia/Playfair Display) weight700可斜体，正文Inter，数字tabular-nums；KPI数值48px weight300 下方2px金色装饰线；卡片纯白 1px描边#e7e5e4 极小圆角6px 无重阴影；对称2列、上下大留白padding40px、区块间1px hairline分隔；图表大地色系#92400e/#b45309/#a8a29e、以细线条图为主、网格隐藏；气质学术期刊配图般的克制优雅' },
  { key: 'noir', emoji: '🎬', label: '黑色电影', prompt: '完全重新设计整个页面视觉风格为【黑色电影】：背景#0a0a0a，黑白灰+唯一点缀红#dc2626；字体Courier/mono 全大写 标题letter-spacing0.12em，数字tabular-nums；KPI数值白色60px weight100极细；卡片#121212 1px描边#262626 纯方角；单列全宽、信息如电影字幕自上而下流动、区块间细线分隔；红色只用于关键/警示数据；图表极简只留1px白/灰线条、无网格无填充；气质黑白电影片头般的高级叙事感' },
];

function StyleDock({ selectedKey, onSelect }: { selectedKey: string | null; onSelect: (key: string, prompt: string) => void }) {
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const [mouseX, setMouseX] = useState(0);
  const dockRef = useRef<HTMLDivElement>(null);

  const handleMouseMove = (e: React.MouseEvent) => {
    if (!dockRef.current) return;
    const rect = dockRef.current.getBoundingClientRect();
    setMouseX(e.clientX - rect.left);
  };

  const getScale = (index: number) => {
    if (hoveredIndex === null) return 1;
    if (!dockRef.current) return 1;
    const itemWidth = dockRef.current.scrollWidth / DOCK_STYLES.length;
    const itemCenter = itemWidth * index + itemWidth / 2;
    const distance = Math.abs(mouseX - itemCenter);
    const maxDistance = itemWidth * 2.5;
    if (distance > maxDistance) return 1;
    return 1 + 0.5 * Math.cos((distance / maxDistance) * Math.PI / 2);
  };

  return (
    <div
      ref={dockRef}
      className="flex items-end gap-0.5 px-2 py-1.5 bg-obsidian-800/80 backdrop-blur-md border border-obsidian-700/50 rounded-xl flex-shrink-0"
      onMouseMove={handleMouseMove}
      onMouseEnter={() => setHoveredIndex(0)}
      onMouseLeave={() => setHoveredIndex(null)}
    >
      {DOCK_STYLES.map((style, i) => {
        const scale = getScale(i);
        const isSelected = selectedKey === style.key;
        return (
          <button
            key={style.key}
            onClick={() => onSelect(style.key, style.prompt)}
            onMouseEnter={() => setHoveredIndex(i)}
            className={`relative flex flex-col items-center transition-transform duration-150 ease-out origin-bottom ${isSelected ? 'opacity-100' : 'opacity-70 hover:opacity-100'}`}
            style={{ transform: `scale(${scale})` }}
            title={style.label}
          >
            <span className="text-base leading-none select-none">{style.emoji}</span>
            {isSelected && <span className="absolute -bottom-1 w-1 h-1 rounded-full bg-amber-500" />}
            {hoveredIndex === i && (
              <span className="absolute -top-6 left-1/2 -translate-x-1/2 bg-obsidian-900 border border-obsidian-700 text-[8px] text-gray-300 px-1.5 py-0.5 rounded whitespace-nowrap shadow-lg z-10">
                {style.label}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

// ── Save-current-report-as-theme modal ──
const THEME_EMOJIS = ['🎨', '🖤', '💚', '🌌', '🔵', '👔', '🟣', '💎', '🌅', '🌿', '📜', '🎬'];

function SaveThemeModal({
  reportId,
  t,
  onClose,
  onSaved,
}: {
  reportId: number;
  t: (k: string, o?: Record<string, unknown>) => string;
  onClose: () => void;
  onSaved: (theme: ReportTheme) => void;
}) {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [stylePrompt, setStylePrompt] = useState('');
  const [emoji, setEmoji] = useState('🎨');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const handleSave = async () => {
    if (!name.trim()) { setErr(t('reportDetail.themes.nameRequired')); return; }
    setBusy(true);
    try {
      // source_report_id makes the backend capture the report's current HTML as
      // the theme's reference template.
      const theme = await reportThemesApi.create({
        name: name.trim(),
        description: description.trim(),
        style_prompt: stylePrompt.trim() || null,
        emoji,
        source_report_id: reportId,
      });
      onSaved(theme);
    } catch (e) {
      setErr(e instanceof Error ? e.message : t('errors.saveFailed'));
    } finally {
      setBusy(false);
    }
  };

  const labelCls = 'block text-[11px] font-medium text-gray-400 mb-1';
  const inputCls = 'w-full bg-obsidian-800 border border-obsidian-700 rounded-md px-2.5 py-1.5 text-xs text-gray-200 focus:border-amber-500/50 focus:outline-none';

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-[420px] shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between mb-4">
          <h3 className="text-sm font-semibold text-gray-100 flex items-center gap-2">
            <Palette size={16} className="text-amber-500" />
            {t('reportDetail.themes.saveTitle')}
          </h3>
          {err && <span className="text-[11px] px-2 py-1 rounded bg-red-500/10 text-red-400">{err}</span>}
        </div>

        <p className="text-[10px] text-gray-500 mb-3">{t('reportDetail.themes.saveHint')}</p>

        <div className="space-y-3">
          <div className="flex gap-3">
            <div>
              <label className={labelCls}>{t('reportDetail.themes.emoji')}</label>
              <div className="flex flex-wrap gap-1 w-24">
                {THEME_EMOJIS.map((e) => (
                  <button
                    key={e}
                    onClick={() => setEmoji(e)}
                    className={`w-6 h-6 rounded flex items-center justify-center text-sm transition-premium ${emoji === e ? 'bg-amber-500/20 ring-1 ring-amber-500/50' : 'hover:bg-obsidian-800'}`}
                  >
                    {e}
                  </button>
                ))}
              </div>
            </div>
            <div className="flex-1">
              <label className={labelCls}>{t('reportDetail.themes.name')}</label>
              <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} placeholder={t('reportDetail.themes.namePlaceholder')} maxLength={120} autoFocus />
              <label className={`${labelCls} mt-2`}>{t('reportDetail.themes.description')}</label>
              <input className={inputCls} value={description} onChange={(e) => setDescription(e.target.value)} placeholder={t('reportDetail.themes.descPlaceholder')} maxLength={200} />
            </div>
          </div>

          <div>
            <label className={labelCls}>{t('reportDetail.themes.stylePrompt')}</label>
            <textarea className={`${inputCls} h-16 resize-none`} value={stylePrompt} onChange={(e) => setStylePrompt(e.target.value)} placeholder={t('reportDetail.themes.stylePromptPlaceholder')} />
            <p className="text-[9px] text-gray-600 mt-1">{t('reportDetail.themes.stylePromptHint')}</p>
          </div>
        </div>

        <div className="flex items-center justify-end gap-2 mt-5">
          <button onClick={onClose} className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium">
            {t('common.cancel')}
          </button>
          <button onClick={handleSave} disabled={busy} className="text-xs text-[#08080c] bg-amber-500 hover:bg-amber-400 px-4 py-1.5 rounded-md font-semibold disabled:opacity-50 transition-premium">
            {t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
