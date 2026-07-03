import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  Star,
  Plus,
  Trash,
  Folder,
  ArrowClockwise,
  X,
  Database,
  Copy,
  PencilSimple,
  Check,
  Play,
  Clock,
} from '@phosphor-icons/react';
import {
  metricsApi,
  metricGroupsApi,
  datasourcesApi,
  queryApi,
  type MetricPool,
  type MetricGroup,
  type DataSource,
} from '../lib/api';
import { PageHeader, ErrorBanner, EmptyState } from '../components/ui';

export default function MetricsPage() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [groups, setGroups] = useState<MetricGroup[]>([]);
  const [datasources, setDatasources] = useState<DataSource[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showNewGroup, setShowNewGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState('');

  // Manual metric creation
  const [showNewMetric, setShowNewMetric] = useState(false);
  const [nmName, setNmName] = useState('');
  const [nmSql, setNmSql] = useState('');
  const [nmDsId, setNmDsId] = useState<number | ''>('');
  const [nmGroupId, setNmGroupId] = useState<number | ''>('');
  const [nmSaving, setNmSaving] = useState(false);
  const [nmTesting, setNmTesting] = useState(false);
  const [nmTestResult, setNmTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  // Selected metric
  const selectedId = searchParams.get('id') ? parseInt(searchParams.get('id')!) : null;
  const selectedMetric = metrics.find((m) => m.id === selectedId) || null;

  // Detail state
  const [detailData, setDetailData] = useState<{ columns: string[]; rows: Record<string, unknown>[] } | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [page, setPage] = useState(1);
  const pageSize = 50;
  const [editingName, setEditingName] = useState(false);
  const [editName, setEditName] = useState('');
  const [editingSql, setEditingSql] = useState(false);
  const [editSql, setEditSql] = useState('');
  const [sqlRunning, setSqlRunning] = useState(false);
  const [sqlDuration, setSqlDuration] = useState<number | null>(null);
  const [sqlError, setSqlError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const [deleteGroupTarget, setDeleteGroupTarget] = useState<MetricGroup | null>(null);

  const handleDeleteGroup = async () => {
    if (!deleteGroupTarget) return;
    try {
      await metricGroupsApi.delete(deleteGroupTarget.id);
      setDeleteGroupTarget(null);
      await fetchAllAndNotify();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.deleteFailed'));
      setDeleteGroupTarget(null);
    }
  };

  const fetchAll = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const [m, g, ds] = await Promise.all([metricsApi.list(), metricGroupsApi.list(), datasourcesApi.list()]);
      setMetrics(m);
      setGroups(g);
      setDatasources(ds);
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  // Notify sidebar to refresh after data changes
  const fetchAllAndNotify = useCallback(async () => {
    await fetchAll();
    window.dispatchEvent(new Event('metrics-updated'));
  }, [fetchAll]);

  useEffect(() => { fetchAll(); }, [fetchAll]);

  // Load detail data when the selected metric changes.
  //
  // The metrics *list* no longer includes the heavy `result_cache`, so we fetch
  // the single metric here to get its cached rows (falling back to its source
  // data pool). Keyed on `selectedId` (a primitive) so this doesn't re-run on
  // every background list refresh.
  useEffect(() => {
    setPage(1); // Reset pagination
    if (!selectedId) {
      setDetailData(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    metricsApi.get(selectedId)
      .then(async (full) => {
        if (cancelled) return;
        if (full.result_cache) {
          const rows = Array.isArray(full.result_cache) ? full.result_cache as Record<string, unknown>[] : [];
          const columns = rows.length > 0 ? Object.keys(rows[0]) : [];
          setDetailData({ columns, rows });
        } else if (full.source_pool_id) {
          const pool = await queryApi.getPool(full.source_pool_id);
          if (cancelled) return;
          if (pool.result_cache) {
            const rows = Array.isArray(pool.result_cache) ? pool.result_cache as Record<string, unknown>[] : [];
            const columns = rows.length > 0 ? Object.keys(rows[0]) : [];
            setDetailData({ columns, rows });
          } else {
            setDetailData(null);
          }
        } else {
          setDetailData(null);
        }
      })
      .catch(() => { if (!cancelled) setDetailData(null); })
      .finally(() => { if (!cancelled) setDetailLoading(false); });
    return () => { cancelled = true; };
  }, [selectedId]);

  const handleCreateGroup = async () => {
    if (!newGroupName.trim()) return;
    try {
      await metricGroupsApi.create({ name: newGroupName.trim() });
      setNewGroupName('');
      setShowNewGroup(false);
      await fetchAllAndNotify();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.createFailed'));
    }
  };

  const openNewMetric = () => {
    setNmName('');
    setNmSql('');
    setNmDsId(datasources[0]?.id ?? '');
    setNmGroupId('');
    setNmTestResult(null);
    setShowNewMetric(true);
  };

  const handleTestMetricSql = async () => {
    if (!nmSql.trim() || !nmDsId) return;
    setNmTesting(true);
    setNmTestResult(null);
    try {
      const result = await queryApi.execute(nmSql.trim(), nmDsId as number);
      setNmTestResult({ ok: true, message: t('metrics.testOk', { count: result.row_count }) });
    } catch (e) {
      setNmTestResult({ ok: false, message: e instanceof Error ? e.message : 'Query failed' });
    } finally {
      setNmTesting(false);
    }
  };

  const handleCreateMetric = async () => {
    if (!nmName.trim() || !nmSql.trim() || !nmDsId) return;
    setNmSaving(true);
    try {
      const created = await metricsApi.create({
        name: nmName.trim(),
        sql_query: nmSql.trim(),
        datasource_id: nmDsId as number,
        group_id: nmGroupId === '' ? null : (nmGroupId as number),
      });
      setShowNewMetric(false);
      await fetchAllAndNotify();
      // Open the newly created metric, then refresh to populate its data
      setSearchParams({ id: String(created.id) });
      metricsApi.refresh(created.id).then(() => fetchAllAndNotify()).catch(() => {});
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.createFailed'));
    } finally {
      setNmSaving(false);
    }
  };

  const handleRefresh = async () => {
    if (!selectedMetric) return;
    try {
      setRefreshing(true);
      const updated = await metricsApi.refresh(selectedMetric.id);
      await fetchAllAndNotify();
      // Update detail data
      if (updated.result_cache) {
        const rows = Array.isArray(updated.result_cache) ? updated.result_cache as Record<string, unknown>[] : [];
        const columns = rows.length > 0 ? Object.keys(rows[0]) : [];
        setDetailData({ columns, rows });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.refreshFailed'));
    } finally {
      setRefreshing(false);
    }
  };

  const handleDelete = async () => {
    if (!selectedMetric) return;
    try {
      await metricsApi.delete(selectedMetric.id);
      setSearchParams({});
      await fetchAllAndNotify();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.deleteFailed'));
    }
  };

  const handleSaveName = async () => {
    if (!selectedMetric || !editName.trim()) { setEditingName(false); return; }
    try {
      await metricsApi.update(selectedMetric.id, { name: editName.trim() });
      setEditingName(false);
      await fetchAllAndNotify();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.saveFailed'));
    }
  };

  const handleSaveSql = async () => {
    if (!selectedMetric || !editSql.trim()) { setEditingSql(false); return; }
    try {
      await metricsApi.update(selectedMetric.id, { sql_query: editSql.trim() });
      setEditingSql(false);
      await fetchAllAndNotify();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.saveFailed'));
    }
  };

  // Run/test the SQL without saving — preview the result
  const handleRunSql = async () => {
    if (!selectedMetric) return;
    const sql = editingSql ? editSql.trim() : selectedMetric.sql_query;
    if (!sql) return;
    setSqlRunning(true);
    setSqlError(null);
    setSqlDuration(null);
    setPage(1);
    const start = performance.now();
    try {
      const result = await queryApi.execute(sql, selectedMetric.datasource_id);
      setSqlDuration(performance.now() - start);
      const rows = result.rows as Record<string, unknown>[];
      const columns = result.columns;
      setDetailData({ columns, rows });
    } catch (e) {
      setSqlDuration(performance.now() - start);
      setSqlError(e instanceof Error ? e.message : 'Query failed');
    } finally {
      setSqlRunning(false);
    }
  };

  // Save SQL and run
  const handleSaveAndRun = async () => {
    await handleSaveSql();
    // After save, selectedMetric will update via fetchAll; run with current editSql
    if (!selectedMetric || !editSql.trim()) return;
    setSqlRunning(true);
    setSqlError(null);
    const start = performance.now();
    try {
      const result = await queryApi.execute(editSql.trim(), selectedMetric.datasource_id);
      setSqlDuration(performance.now() - start);
      setDetailData({ columns: result.columns, rows: result.rows as Record<string, unknown>[] });
    } catch (e) {
      setSqlDuration(performance.now() - start);
      setSqlError(e instanceof Error ? e.message : 'Query failed');
    } finally {
      setSqlRunning(false);
    }
  };

  const handleCopySql = () => {
    if (!selectedMetric) return;
    navigator.clipboard.writeText(selectedMetric.sql_query).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  const groupName = selectedMetric?.group_id
    ? groups.find((g) => g.id === selectedMetric.group_id)?.name || t('metrics.ungrouped')
    : t('metrics.ungrouped');

  // ── No metric selected ──
  if (!selectedId) {
    return (
      <div className="p-6">
        <PageHeader
          title={t('metrics.title')}
          description={t('metrics.description')}
          action={
            <div className="flex items-center gap-2">
              <button
                onClick={openNewMetric}
                disabled={datasources.length === 0}
                title={datasources.length === 0 ? t('metrics.needDatasource') : undefined}
                className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-3 py-2 rounded-lg transition-premium active:translate-y-[1px] disabled:opacity-50 disabled:cursor-not-allowed"
              >
                <Plus size={14} weight="bold" />
                {t('metrics.newMetric')}
              </button>
              <button
                onClick={() => setShowNewGroup(true)}
                className="flex items-center gap-1.5 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-200 text-xs px-3 py-2 rounded-lg transition-premium active:translate-y-[1px]"
              >
                <Folder size={14} />
                {t('metrics.newGroup')}
              </button>
            </div>
          }
        />
        {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

        {loading && (
          <div className="flex items-center justify-center py-16">
            <div className="flex items-center gap-2 text-xs text-gray-500">
              <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
              {t('common.loading')}
            </div>
          </div>
        )}

        {!loading && showNewGroup && (
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-4 mb-4 flex items-center gap-3">
            <Folder size={16} className="text-amber-500 flex-shrink-0" />
            <input
              type="text"
              value={newGroupName}
              onChange={(e) => setNewGroupName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreateGroup(); }}
              placeholder={t('metrics.groupNamePlaceholder')}
              autoFocus
              className="flex-1 bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
            />
            <button onClick={handleCreateGroup} className="text-xs text-amber-500 hover:text-amber-400 font-medium">{t('common.save')}</button>
            <button onClick={() => setShowNewGroup(false)} className="text-gray-500 hover:text-gray-300"><X size={14} /></button>
          </div>
        )}

        {/* Group list with delete */}
        {!loading && groups.length > 0 && (
          <div className="mb-6 space-y-1">
            {groups.map((group) => {
              const count = metrics.filter((m) => m.group_id === group.id).length;
              return (
                <div key={group.id} className="flex items-center gap-2 bg-obsidian-900 border border-obsidian-700 rounded-lg px-3 py-2 group">
                  <Folder size={14} className="text-amber-500/60 flex-shrink-0" />
                  <span className="text-xs text-gray-300 font-medium flex-1 truncate">{group.name}</span>
                  <span className="text-[9px] text-gray-600">{count}</span>
                  <button
                    onClick={() => setDeleteGroupTarget(group)}
                    className="text-gray-600 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-premium"
                    title={t('common.delete')}
                  >
                    <Trash size={12} />
                  </button>
                </div>
              );
            })}
          </div>
        )}

        {!loading && (
          <EmptyState
            icon={Star}
            title={t('metrics.empty.selectMetric')}
            description={t('metrics.empty.selectMetricDesc')}
          />
        )}

        {/* New Metric Modal */}
        {showNewMetric && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={() => setShowNewMetric(false)}>
            <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl w-full max-w-lg flex flex-col max-h-[85vh]" onClick={(e) => e.stopPropagation()}>
              <div className="flex items-center justify-between px-5 py-3 border-b border-obsidian-700 flex-shrink-0">
                <h3 className="text-sm font-semibold text-gray-100 flex items-center gap-2">
                  <Star size={15} className="text-amber-500" weight="fill" />
                  {t('metrics.newMetric')}
                </h3>
                <button onClick={() => setShowNewMetric(false)} className="text-gray-500 hover:text-gray-200 transition-premium">
                  <X size={16} />
                </button>
              </div>

              <div className="overflow-y-auto scrollbar-thin p-5 space-y-3">
                {/* Name */}
                <div>
                  <label className="text-[10px] text-gray-500 block mb-1">{t('metrics.metricName')}</label>
                  <input
                    type="text"
                    value={nmName}
                    onChange={(e) => setNmName(e.target.value)}
                    placeholder={t('metrics.metricNamePlaceholder')}
                    autoFocus
                    className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
                  />
                </div>

                {/* Datasource + Group */}
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="text-[10px] text-gray-500 block mb-1">{t('metrics.datasource')}</label>
                    <select
                      value={nmDsId}
                      onChange={(e) => { setNmDsId(e.target.value ? parseInt(e.target.value) : ''); setNmTestResult(null); }}
                      className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50 transition-premium"
                    >
                      {datasources.map((ds) => (
                        <option key={ds.id} value={ds.id}>{ds.name} ({ds.db_type})</option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label className="text-[10px] text-gray-500 block mb-1">{t('metrics.group')}</label>
                    <select
                      value={nmGroupId}
                      onChange={(e) => setNmGroupId(e.target.value ? parseInt(e.target.value) : '')}
                      className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50 transition-premium"
                    >
                      <option value="">{t('metrics.ungrouped')}</option>
                      {groups.map((g) => (
                        <option key={g.id} value={g.id}>{g.name}</option>
                      ))}
                    </select>
                  </div>
                </div>

                {/* SQL */}
                <div>
                  <label className="text-[10px] text-gray-500 block mb-1">SQL</label>
                  <textarea
                    value={nmSql}
                    onChange={(e) => { setNmSql(e.target.value); setNmTestResult(null); }}
                    rows={6}
                    placeholder="SELECT ..."
                    className="w-full bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-[11px] text-data-green font-mono focus:outline-none focus:border-amber-500/50 transition-premium resize-y"
                  />
                </div>

                {/* Test result */}
                {nmTestResult && (
                  <div className={`text-[10px] px-3 py-2 rounded-lg border ${
                    nmTestResult.ok
                      ? 'text-data-green bg-data-green/10 border-data-green/20'
                      : 'text-red-400 bg-red-500/10 border-red-500/20'
                  }`}>
                    {nmTestResult.ok ? '✓ ' : '✗ '}{nmTestResult.message}
                  </div>
                )}
              </div>

              <div className="flex items-center justify-between px-5 py-3 border-t border-obsidian-700 flex-shrink-0">
                <button
                  onClick={handleTestMetricSql}
                  disabled={!nmSql.trim() || !nmDsId || nmTesting}
                  className="flex items-center gap-1 text-[11px] text-gray-300 hover:text-gray-100 border border-obsidian-700 px-3 py-1.5 rounded-md transition-premium disabled:opacity-40"
                >
                  <Play size={11} weight="fill" />
                  {nmTesting ? '...' : t('metrics.test')}
                </button>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => setShowNewMetric(false)}
                    className="text-[11px] text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
                  >
                    {t('common.cancel')}
                  </button>
                  <button
                    onClick={handleCreateMetric}
                    disabled={!nmName.trim() || !nmSql.trim() || !nmDsId || nmSaving}
                    className="text-[11px] text-[#08080c] font-medium bg-amber-500 hover:bg-amber-400 px-4 py-1.5 rounded-md transition-premium disabled:opacity-50"
                  >
                    {nmSaving ? t('common.loading') : t('common.save')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Delete Group Confirmation Modal */}
        {deleteGroupTarget && (
          <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setDeleteGroupTarget(null)}>
            <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl" onClick={(e) => e.stopPropagation()}>
              <h3 className="text-sm font-semibold text-gray-100 mb-2">{t('metrics.deleteGroupConfirm.title')}</h3>
              <p className="text-xs text-gray-400 mb-4">{t('metrics.deleteGroupConfirm.message', { name: deleteGroupTarget.name })}</p>
              <div className="flex items-center justify-end gap-2">
                <button
                  onClick={() => setDeleteGroupTarget(null)}
                  className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
                >
                  {t('common.cancel')}
                </button>
                <button
                  onClick={handleDeleteGroup}
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

  // ── Metric detail view ──
  return (
    <div className="p-6 h-full flex flex-col">
      {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

      {/* Header */}
      <div className="flex-shrink-0 mb-4">
        <div className="flex items-center justify-between">
          <div className="min-w-0 flex-1">
            {editingName ? (
              <div className="flex items-center gap-2">
                <input
                  value={editName}
                  onChange={(e) => setEditName(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleSaveName(); if (e.key === 'Escape') setEditingName(false); }}
                  autoFocus
                  className="bg-obsidian-800 border border-amber-500/50 rounded-lg px-3 py-1.5 text-sm font-bold text-gray-100 focus:outline-none"
                />
                <button onClick={handleSaveName} className="text-amber-500 hover:text-amber-400"><Check size={16} /></button>
                <button onClick={() => setEditingName(false)} className="text-gray-500 hover:text-gray-300"><X size={16} /></button>
              </div>
            ) : (
              <h1
                className="text-lg font-bold text-gray-100 tracking-tight cursor-pointer hover:text-amber-500 transition-premium"
                onDoubleClick={() => { setEditName(selectedMetric?.name || ''); setEditingName(true); }}
                title="Double-click to rename"
              >
                <Star size={16} className="text-amber-500 inline mr-2" weight="fill" />
                {selectedMetric?.name || '—'}
              </h1>
            )}
            <div className="flex items-center gap-3 mt-1 text-[10px] text-gray-500">
              <span className="flex items-center gap-1">
                <Folder size={10} /> {groupName}
              </span>
              <span className="flex items-center gap-1">
                <Database size={10} /> {datasources.find((d) => d.id === selectedMetric?.datasource_id)?.name || `DS#${selectedMetric?.datasource_id}`}
              </span>
              {selectedMetric?.row_count != null && (
                <span>{selectedMetric.row_count.toLocaleString()} rows</span>
              )}
              {selectedMetric?.created_at && (
                <span className="flex items-center gap-1" title={new Date(selectedMetric.created_at).toLocaleString()}>
                  <Clock size={10} /> {t('metrics.favoritedAt')} {new Date(selectedMetric.created_at).toLocaleDateString()}
                </span>
              )}
            </div>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={handleRefresh}
              disabled={refreshing}
              className="flex items-center gap-1.5 text-[10px] text-amber-500 hover:text-amber-400 bg-amber-500/5 hover:bg-amber-500/10 border border-amber-500/20 px-2.5 py-1.5 rounded-md transition-premium disabled:opacity-50"
            >
              <ArrowClockwise size={12} className={refreshing ? 'animate-spin' : ''} />
              {t('common.refresh')}
            </button>
            <button
              onClick={handleDelete}
              className="flex items-center gap-1.5 text-[10px] text-gray-500 hover:text-red-400 border border-obsidian-700 hover:border-red-500/30 px-2.5 py-1.5 rounded-md transition-premium"
            >
              <Trash size={12} />
              {t('common.delete')}
            </button>
          </div>
        </div>
      </div>

      {/* SQL Section */}
      <div className="flex-shrink-0 mb-4">
        <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden">
          <div className="flex items-center justify-between px-4 py-2 border-b border-obsidian-700">
            <span className="text-[10px] text-gray-500 uppercase tracking-wide font-medium">SQL</span>
            <div className="flex items-center gap-1.5">
              <button
                onClick={handleCopySql}
                className="flex items-center gap-1 text-[9px] text-gray-500 hover:text-gray-300 px-1.5 py-0.5 rounded transition-premium"
              >
                {copied ? <Check size={10} className="text-data-green" /> : <Copy size={10} />}
                {copied ? t('metrics.copied') : t('metrics.copySql')}
              </button>
              {!editingSql && (
                <button
                  onClick={() => { setEditSql(selectedMetric?.sql_query || ''); setEditingSql(true); }}
                  className="flex items-center gap-1 text-[9px] text-gray-500 hover:text-gray-300 px-1.5 py-0.5 rounded transition-premium"
                >
                  <PencilSimple size={10} />
                  {t('metrics.editSql')}
                </button>
              )}
              <button
                onClick={handleRunSql}
                disabled={sqlRunning}
                className="flex items-center gap-1 text-[9px] bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold px-2 py-0.5 rounded transition-premium disabled:opacity-40"
              >
                <Play size={9} weight="fill" />
                {sqlRunning ? '...' : t('metrics.run')}
              </button>
            </div>
          </div>
          {editingSql ? (
            <div className="p-3">
              <textarea
                value={editSql}
                onChange={(e) => setEditSql(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); handleRunSql(); } }}
                rows={4}
                className="w-full bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-data-green font-mono focus:outline-none focus:border-amber-500/50 transition-premium resize-y"
                placeholder="Ctrl+Enter to run"
              />
              <div className="flex items-center gap-2 mt-2">
                <button onClick={handleSaveAndRun} className="flex items-center gap-1 text-[10px] text-amber-500 hover:text-amber-400 font-medium">
                  <Check size={10} /> {t('metrics.saveAndRun')}
                </button>
                <button onClick={handleSaveSql} className="text-[10px] text-gray-400 hover:text-gray-200">{t('common.save')}</button>
                <button onClick={() => setEditingSql(false)} className="text-[10px] text-gray-500 hover:text-gray-300">{t('common.cancel')}</button>
              </div>
            </div>
          ) : (
            <div className="p-3">
              <code className="text-xs text-data-green font-mono whitespace-pre-wrap break-all block">
                {selectedMetric?.sql_query}
              </code>
            </div>
          )}
          {/* Execution info */}
          {(sqlDuration !== null || sqlError) && (
            <div className="px-4 py-1.5 border-t border-obsidian-700/50 flex items-center gap-3">
              {sqlDuration !== null && (
                <span className="flex items-center gap-1 text-[9px] text-gray-500">
                  <Clock size={9} /> {sqlDuration < 1000 ? `${Math.round(sqlDuration)}ms` : `${(sqlDuration / 1000).toFixed(1)}s`}
                </span>
              )}
              {sqlError && (
                <span className="text-[9px] text-red-400 truncate flex-1" title={sqlError}>✗ {sqlError}</span>
              )}
            </div>
          )}
        </div>
      </div>

      {/* Data Table */}
      <div className="flex-1 bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden min-h-0 flex flex-col">
        <div className="flex items-center justify-between px-4 py-2 border-b border-obsidian-700 flex-shrink-0">
          <span className="text-[10px] text-gray-500 uppercase tracking-wide font-medium">
            {t('metrics.resultData')}
          </span>
          {detailData && (
            <span className="text-[9px] text-gray-600">
              {detailData.rows.length.toLocaleString()} rows · {detailData.columns.length} columns
            </span>
          )}
        </div>

        <div className="flex-1 overflow-auto scrollbar-thin">
          {detailLoading && (
            <div className="flex items-center justify-center py-12">
              <div className="flex items-center gap-2 text-xs text-gray-500">
                <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
                {t('common.loading')}
              </div>
            </div>
          )}

          {!detailLoading && !detailData && (
            <div className="flex flex-col items-center justify-center py-12">
              <Database size={24} className="text-gray-700 mb-2" />
              <p className="text-xs text-gray-600">{t('metrics.noData')}</p>
              <button
                onClick={handleRefresh}
                disabled={refreshing}
                className="mt-3 flex items-center gap-1.5 text-xs text-amber-500 hover:text-amber-400 bg-amber-500/5 hover:bg-amber-500/10 border border-amber-500/20 px-3 py-1.5 rounded-lg transition-premium"
              >
                <ArrowClockwise size={12} />
                {t('metrics.runQuery')}
              </button>
            </div>
          )}

          {detailData && detailData.rows.length > 0 && (() => {
            const totalRows = detailData.rows.length;
            const totalPages = Math.ceil(totalRows / pageSize);
            const startIdx = (page - 1) * pageSize;
            const pageRows = detailData.rows.slice(startIdx, startIdx + pageSize);

            return (
              <>
                <table className="w-full text-[10px]">
                  <thead className="sticky top-0 bg-obsidian-900 z-10">
                    <tr className="border-b border-obsidian-700">
                      {detailData.columns.map((col) => (
                        <th key={col} className="px-3 py-2 text-left text-gray-500 font-medium font-mono whitespace-nowrap">
                          {col}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {pageRows.map((row, ri) => (
                      <tr key={ri} className="border-b border-obsidian-700/30 hover:bg-obsidian-800/30">
                        {detailData.columns.map((col) => (
                          <td key={col} className="px-3 py-1.5 text-gray-300 font-mono whitespace-nowrap">
                            {String(row[col] ?? '')}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>

                {/* Pagination */}
                {totalPages > 1 && (
                  <div className="sticky bottom-0 bg-obsidian-900 border-t border-obsidian-700 px-4 py-2 flex items-center justify-between">
                    <span className="text-[9px] text-gray-600">
                      {startIdx + 1}–{Math.min(startIdx + pageSize, totalRows)} / {totalRows.toLocaleString()}
                    </span>
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => setPage(1)}
                        disabled={page === 1}
                        className="text-[9px] text-gray-500 hover:text-gray-300 disabled:text-gray-700 px-1.5 py-0.5 rounded transition-premium"
                      >
                        «
                      </button>
                      <button
                        onClick={() => setPage((p) => Math.max(1, p - 1))}
                        disabled={page === 1}
                        className="text-[9px] text-gray-500 hover:text-gray-300 disabled:text-gray-700 px-1.5 py-0.5 rounded transition-premium"
                      >
                        ‹
                      </button>
                      <span className="text-[9px] text-gray-400 px-2">
                        {page} / {totalPages}
                      </span>
                      <button
                        onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                        disabled={page === totalPages}
                        className="text-[9px] text-gray-500 hover:text-gray-300 disabled:text-gray-700 px-1.5 py-0.5 rounded transition-premium"
                      >
                        ›
                      </button>
                      <button
                        onClick={() => setPage(totalPages)}
                        disabled={page === totalPages}
                        className="text-[9px] text-gray-500 hover:text-gray-300 disabled:text-gray-700 px-1.5 py-0.5 rounded transition-premium"
                      >
                        »
                      </button>
                    </div>
                  </div>
                )}
              </>
            );
          })()}

          {detailData && detailData.rows.length === 0 && (
            <div className="flex items-center justify-center py-12">
              <p className="text-xs text-gray-600">{t('metrics.emptyResult')}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
