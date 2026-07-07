import { useEffect, useState, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Camera, Plus, Trash, Clock, ArrowUp, ArrowDown, Minus, List } from '@phosphor-icons/react';
import { useSnapshotStore } from '../stores/snapshotStore';
import { metricsApi } from '../lib/api';
import type { MetricPool, MetricSnapshot, SnapshotSchedule } from '../lib/types';
import { ConfirmDialog } from '../components/ui';

export default function SnapshotsPage() {
  const { t } = useTranslation();
  const {
    schedules,
    snapshots,
    loading,
    fetchSchedules,
    fetchSnapshots,
    createSchedule,
    updateSchedule,
    deleteSchedule,
    takeSnapshot,
    deleteSnapshot,
  } = useSnapshotStore();

  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [showCreate, setShowCreate] = useState(false);
  const [selectedSchedule, setSelectedSchedule] = useState<SnapshotSchedule | null>(null);
  const [selectedMetricId, setSelectedMetricId] = useState<number | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<{ type: 'schedule' | 'snapshot'; id: number } | null>(null);
  const [toggleTarget, setToggleTarget] = useState<SnapshotSchedule | null>(null);
  const [currentPage, setCurrentPage] = useState(1);
  const PAGE_SIZE = 20;
  // Mobile: the schedule list is an overlay drawer toggled by a button.
  const [showList, setShowList] = useState(false);

  // Form state for creating schedule
  const [formMetricId, setFormMetricId] = useState<number | ''>();
  const [formType, setFormType] = useState('daily');
  const [formCron, setFormCron] = useState('');
  const [formRetention, setFormRetention] = useState('');

  useEffect(() => {
    fetchSchedules();
    metricsApi.list().then(setMetrics).catch(() => {});
  }, [fetchSchedules]);

  // Periodically refresh schedules to keep enabled/disabled state in sync
  useEffect(() => {
    const interval = setInterval(() => {
      fetchSchedules();
    }, 10000);
    return () => clearInterval(interval);
  }, [fetchSchedules]);

  const handleCreateSchedule = async () => {
    if (!formMetricId) return;
    try {
      await createSchedule({
        metric_pool_id: formMetricId as number,
        schedule_type: formType,
        cron_expr: formType === 'cron' ? formCron : null,
        retention_days: formRetention ? parseInt(formRetention) : null,
      });
      setShowCreate(false);
      setFormMetricId(undefined);
      setFormType('daily');
      setFormCron('');
      setFormRetention('');
    } catch {}
  };

  const handleSelectSchedule = (schedule: SnapshotSchedule) => {
    setSelectedSchedule(schedule);
    setSelectedMetricId(schedule.metric_pool_id);
    setCurrentPage(1);
    fetchSnapshots(schedule.metric_pool_id, { limit: 200 });
  };

  // Auto-refresh snapshots when a schedule is selected and enabled
  useEffect(() => {
    if (!selectedSchedule || !selectedSchedule.enabled || !selectedMetricId) return;
    const interval = setInterval(() => {
      fetchSnapshots(selectedMetricId, { limit: 200 });
    }, 5000);
    return () => clearInterval(interval);
  }, [selectedSchedule, selectedMetricId, fetchSnapshots]);

  const handleToggleEnabled = async (schedule: SnapshotSchedule) => {
    setToggleTarget(schedule);
  };

  const confirmToggle = async () => {
    if (!toggleTarget) return;
    try {
      const updated = await updateSchedule(toggleTarget.id, { enabled: !toggleTarget.enabled });
      if (selectedSchedule?.id === toggleTarget.id) {
        setSelectedSchedule(updated);
      }
    } catch (e) {
      console.error('Toggle failed:', e);
    }
    setToggleTarget(null);
    await fetchSchedules();
  };

  const handleDelete = async (id: number) => {
    setDeleteTarget({ type: 'schedule', id });
  };

  const handleDeleteSnapshot = (_metricId: number, snapshotId: number) => {
    setDeleteTarget({ type: 'snapshot', id: snapshotId });
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      if (deleteTarget.type === 'schedule') {
        await deleteSchedule(deleteTarget.id);
        if (selectedSchedule?.id === deleteTarget.id) {
          setSelectedSchedule(null);
          setSelectedMetricId(null);
        }
      } else {
        if (selectedMetricId) {
          await deleteSnapshot(selectedMetricId, deleteTarget.id);
        }
      }
    } catch {}
    setDeleteTarget(null);
  };

  const handleTakeSnapshot = async () => {
    if (!selectedMetricId) return;
    await takeSnapshot(selectedMetricId);
  };

  const getMetricName = (metricId: number) => {
    return metrics.find((m) => m.id === metricId)?.name || `Metric #${metricId}`;
  };

  const formatTime = (ts?: string | null) => {
    if (!ts) return t('snapshots.never');
    return new Date(ts).toLocaleString();
  };

  return (
    <div className="flex h-full overflow-hidden relative">
      {/* Mobile overlay when the schedule list drawer is open */}
      {showList && (
        <div
          className="md:hidden absolute inset-0 z-30 bg-black/60"
          onClick={() => setShowList(false)}
        />
      )}
      {/* Left: Schedule List */}
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
              <Camera size={16} className="text-amber-500" />
              {t('snapshots.title')}
            </h2>
            <button
              onClick={() => setShowCreate(true)}
              className="w-6 h-6 rounded flex items-center justify-center text-gray-400 hover:text-amber-500 hover:bg-obsidian-800 transition-colors"
            >
              <Plus size={14} />
            </button>
          </div>
          <p className="text-[10px] text-gray-500">{t('snapshots.description')}</p>
        </div>

        {/* Schedule Items */}
        <div className="flex-1 overflow-y-auto scrollbar-thin p-2 space-y-1">
          {schedules.length === 0 && !loading && (
            <div className="text-center py-8 px-4">
              <Camera size={32} className="mx-auto text-gray-600 mb-2" />
              <p className="text-xs text-gray-500">{t('snapshots.noSchedules')}</p>
              <p className="text-[10px] text-gray-600 mt-1">{t('snapshots.noSchedulesHint')}</p>
            </div>
          )}
          {schedules.map((schedule) => (
            <div
              key={schedule.id}
              onClick={() => { handleSelectSchedule(schedule); setShowList(false); }}
              className={`p-2.5 rounded-lg cursor-pointer transition-colors border ${
                selectedSchedule?.id === schedule.id
                  ? 'bg-amber-500/5 border-amber-500/20'
                  : 'border-transparent hover:bg-obsidian-800'
              }`}
            >
              <div className="flex items-center justify-between">
                <span className="text-xs text-gray-200 font-medium truncate">
                  {getMetricName(schedule.metric_pool_id)}
                  <span className="text-[9px] text-gray-500 ml-1.5">#{schedule.metric_pool_id}</span>
                </span>
                <button
                  onClick={(e) => { e.stopPropagation(); handleDelete(schedule.id); }}
                  className="w-5 h-5 rounded flex items-center justify-center text-gray-500 hover:text-red-400"
                  aria-label={t('common.delete')}
                  title={t('common.delete')}
                >
                  <Trash size={10} />
                </button>
              </div>
              <div className="flex items-center gap-2 mt-1">
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-obsidian-700 text-gray-400">
                  {t(`snapshots.${schedule.schedule_type}`)}
                </span>
                <button
                  onClick={(e) => { e.stopPropagation(); handleToggleEnabled(schedule); }}
                  className={`px-1.5 py-0.5 rounded text-[9px] font-medium transition-colors ${
                    schedule.enabled
                      ? 'bg-green-500/15 text-green-400 hover:bg-green-500/25'
                      : 'bg-gray-500/15 text-gray-500 hover:bg-gray-500/25'
                  }`}
                >
                  {schedule.enabled ? '运行中' : '已暂停'}
                </button>
              </div>
              <div className="text-[9px] text-gray-600 mt-1 flex items-center gap-1">
                <Clock size={9} />
                {t('snapshots.nextRun')}: {formatTime(schedule.next_run_at)}
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Right: Snapshot Data Table */}
      <div className="flex-1 flex flex-col overflow-hidden">
        {selectedSchedule ? (
          <>
            {/* Header */}
            <div className="p-4 border-b border-obsidian-700 flex items-center justify-between flex-shrink-0 gap-2">
              <div className="flex items-center gap-2 min-w-0">
                <button
                  onClick={() => setShowList(true)}
                  className="md:hidden w-8 h-8 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-200 hover:bg-obsidian-800 flex-shrink-0"
                  aria-label={t('snapshots.title')}
                >
                  <List size={18} />
                </button>
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold text-gray-200 truncate">
                    {getMetricName(selectedSchedule.metric_pool_id)}
                    <span className="text-[10px] text-gray-500 font-normal ml-2">ID: {selectedSchedule.metric_pool_id}</span>
                  </h3>
                  <p className="text-[10px] text-gray-500 mt-0.5">
                    {t('snapshots.lastRun')}: {formatTime(selectedSchedule.last_run_at)} · {t(`snapshots.${selectedSchedule.schedule_type}`)}
                    {snapshots.length > 0 && ` · ${snapshots.length} 次采集`}
                  </p>
                </div>
              </div>
              <button
                onClick={handleTakeSnapshot}
                disabled={loading}
                className="px-2.5 py-1.5 rounded-md bg-amber-500/10 text-amber-500 text-[11px] font-medium hover:bg-amber-500/20 transition-colors disabled:opacity-50"
              >
                <Camera size={12} className="inline mr-1" />
                {t('snapshots.takeNow')}
              </button>
            </div>

            {/* Table Content */}
            <div className="flex-1 overflow-auto p-4">
              {snapshots.length === 0 ? (
                <div className="text-center py-12">
                  <Camera size={36} className="mx-auto text-gray-700 mb-2" />
                  <p className="text-xs text-gray-500">{t('snapshots.noSnapshots')}</p>
                </div>
              ) : (
                <>
                  <SnapshotDataTable
                    snapshots={snapshots.slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE)}
                    allSnapshots={snapshots}
                    pageOffset={(currentPage - 1) * PAGE_SIZE}
                    metricId={selectedMetricId!}
                    onDelete={handleDeleteSnapshot}
                  />
                  {/* Pagination */}
                  {snapshots.length > PAGE_SIZE && (
                    <div className="flex items-center justify-between mt-4 px-1">
                      <span className="text-[10px] text-gray-500">
                        共 {snapshots.length} 条 · 第 {currentPage}/{Math.ceil(snapshots.length / PAGE_SIZE)} 页
                      </span>
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setCurrentPage((p) => Math.max(1, p - 1))}
                          disabled={currentPage === 1}
                          className="px-2 py-1 rounded text-[10px] text-gray-400 hover:text-gray-200 hover:bg-obsidian-800 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                        >
                          上一页
                        </button>
                        {Array.from({ length: Math.min(7, Math.ceil(snapshots.length / PAGE_SIZE)) }, (_, i) => {
                          const totalPages = Math.ceil(snapshots.length / PAGE_SIZE);
                          let page: number;
                          if (totalPages <= 7) {
                            page = i + 1;
                          } else if (currentPage <= 4) {
                            page = i + 1;
                          } else if (currentPage >= totalPages - 3) {
                            page = totalPages - 6 + i;
                          } else {
                            page = currentPage - 3 + i;
                          }
                          return (
                            <button
                              key={page}
                              onClick={() => setCurrentPage(page)}
                              className={`w-6 h-6 rounded text-[10px] transition-colors ${
                                currentPage === page
                                  ? 'bg-amber-500/20 text-amber-500 font-medium'
                                  : 'text-gray-500 hover:text-gray-200 hover:bg-obsidian-800'
                              }`}
                            >
                              {page}
                            </button>
                          );
                        })}
                        <button
                          onClick={() => setCurrentPage((p) => Math.min(Math.ceil(snapshots.length / PAGE_SIZE), p + 1))}
                          disabled={currentPage >= Math.ceil(snapshots.length / PAGE_SIZE)}
                          className="px-2 py-1 rounded text-[10px] text-gray-400 hover:text-gray-200 hover:bg-obsidian-800 disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                        >
                          下一页
                        </button>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 flex items-center justify-center">
            <div className="text-center">
              <Camera size={48} className="mx-auto text-gray-700 mb-3" />
              <p className="text-sm text-gray-500">{t('snapshots.noSchedules')}</p>
              <p className="text-[10px] text-gray-600 mt-1">{t('snapshots.noSchedulesHint')}</p>
              <button
                onClick={() => setShowList(true)}
                className="md:hidden mt-4 inline-flex items-center gap-1.5 bg-amber-500/10 text-amber-500 text-xs px-3 py-2 rounded-lg"
              >
                <List size={14} /> {t('snapshots.title')}
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Create Schedule Modal */}
      {showCreate && (
        <CreateScheduleModal
          metrics={metrics}
          formMetricId={formMetricId}
          formType={formType}
          formCron={formCron}
          formRetention={formRetention}
          onMetricChange={setFormMetricId}
          onTypeChange={setFormType}
          onCronChange={setFormCron}
          onRetentionChange={setFormRetention}
          onSubmit={handleCreateSchedule}
          onClose={() => setShowCreate(false)}
          loading={loading}
          t={t}
        />
      )}

      {/* Delete Confirmation Modal */}
      <ConfirmDialog
        open={!!deleteTarget}
        title={t('snapshots.deleteConfirm.title')}
        message={t('snapshots.deleteConfirm.message')}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />

      {/* Toggle Confirmation Modal */}
      {toggleTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setToggleTarget(null)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-sm font-semibold text-gray-100 mb-2">
              {toggleTarget.enabled ? '确认暂停' : '确认启动'}
            </h3>
            <p className="text-xs text-gray-400 mb-4">
              {toggleTarget.enabled
                ? `确定要暂停「${getMetricName(toggleTarget.metric_pool_id)}」的定时快照吗？`
                : `确定要启动「${getMetricName(toggleTarget.metric_pool_id)}」的定时快照吗？`
              }
            </p>
            <div className="flex items-center justify-end gap-2">
              <button
                onClick={() => setToggleTarget(null)}
                className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={confirmToggle}
                className={`text-xs text-white px-3 py-1.5 rounded-md transition-premium ${
                  toggleTarget.enabled
                    ? 'bg-amber-600 hover:bg-amber-500'
                    : 'bg-green-600 hover:bg-green-500'
                }`}
              >
                {toggleTarget.enabled ? '暂停' : '启动'}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Snapshot Data Table ──
// Renders snapshots as a table where:
// - Each row is a snapshot (time period)
// - Columns are the data fields from result_data
// - Shows change indicators (up/down arrows) between consecutive snapshots
function SnapshotDataTable({
  snapshots,
  allSnapshots,
  pageOffset,
  metricId,
  onDelete,
}: {
  snapshots: MetricSnapshot[];
  allSnapshots: MetricSnapshot[];
  pageOffset: number;
  metricId: number;
  onDelete: (metricId: number, snapshotId: number) => void;
}) {
  // Parse snapshots - they're sorted newest first from API
  const parsed = useMemo(() => {
    return snapshots.map((snap) => {
      const data = snap.result_data as Record<string, unknown>[] | unknown;
      const rows = Array.isArray(data) ? data as Record<string, unknown>[] : [];
      return { ...snap, rows };
    });
  }, [snapshots]);

  // Parse all snapshots for cross-page change calculation
  const allParsed = useMemo(() => {
    return allSnapshots.map((snap) => {
      const data = snap.result_data as Record<string, unknown>[] | unknown;
      const rows = Array.isArray(data) ? data as Record<string, unknown>[] : [];
      return { ...snap, rows };
    });
  }, [allSnapshots]);

  // Extract columns from the first non-empty snapshot
  const columns = useMemo(() => {
    for (const snap of parsed) {
      if (snap.rows.length > 0) {
        return Object.keys(snap.rows[0]);
      }
    }
    return [];
  }, [parsed]);

  // For single-row results (aggregates like SUM, COUNT, etc),
  // show a "time series" view: each snapshot is a row, columns are the fields
  const isSingleRow = parsed.every((s) => s.rows.length <= 1);

  if (columns.length === 0) {
    return <p className="text-xs text-gray-500">暂无可展示的数据</p>;
  }

  if (isSingleRow) {
    return (
      <SingleRowTable
        parsed={parsed}
        allParsed={allParsed}
        pageOffset={pageOffset}
        columns={columns}
        metricId={metricId}
        onDelete={onDelete}
      />
    );
  }

  // Multi-row results: show each snapshot as an expandable section
  return (
    <MultiRowTable
      parsed={parsed}
      allParsed={allParsed}
      pageOffset={pageOffset}
      columns={columns}
      metricId={metricId}
      onDelete={onDelete}
    />
  );
}

// ── Single-row table view (most common for KPIs) ──
// One row per snapshot, with columns for each field value
function SingleRowTable({
  parsed,
  allParsed,
  pageOffset,
  columns,
  metricId,
  onDelete,
}: {
  parsed: { id: number; period_key: string; snapshot_at: string; rows: Record<string, unknown>[]; row_count?: number }[];
  allParsed: { id: number; period_key: string; snapshot_at: string; rows: Record<string, unknown>[]; row_count?: number }[];
  pageOffset: number;
  columns: string[];
  metricId: number;
  onDelete: (metricId: number, snapshotId: number) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="rounded-lg border border-obsidian-700 overflow-hidden">
      <div className="overflow-x-auto">
        <table className="w-full text-left">
          <thead>
            <tr className="bg-obsidian-800/60">
              <th className="px-3 py-2 text-[10px] text-gray-400 font-semibold uppercase tracking-wider border-b border-obsidian-700 sticky left-0 bg-obsidian-800/60 z-10">
                周期
              </th>
              <th className="px-3 py-2 text-[10px] text-gray-400 font-semibold uppercase tracking-wider border-b border-obsidian-700">
                采集时间
              </th>
              {columns.map((col) => (
                <th key={col} className="px-3 py-2 text-[10px] text-gray-400 font-semibold uppercase tracking-wider border-b border-obsidian-700">
                  {col}
                </th>
              ))}
              <th className="px-2 py-2 border-b border-obsidian-700 w-8" />
            </tr>
          </thead>
          <tbody>
            {parsed.map((snap, idx) => {
              const row = snap.rows[0] || {};
              // Use allParsed for previous comparison (handles cross-page boundary)
              const globalIdx = pageOffset + idx;
              const prevSnap = allParsed[globalIdx + 1];
              const prevRow = prevSnap?.rows[0] || {};

              return (
                <tr
                  key={snap.id}
                  className="border-b border-obsidian-800 hover:bg-obsidian-800/30 transition-colors group"
                >
                  <td className="px-3 py-2 sticky left-0 bg-obsidian-950 group-hover:bg-obsidian-800/30 z-10">
                    <span className="text-xs text-gray-200 font-mono">{snap.period_key}</span>
                  </td>
                  <td className="px-3 py-2">
                    <span className="text-[10px] text-gray-500">
                      {new Date(snap.snapshot_at).toLocaleString()}
                    </span>
                  </td>
                  {columns.map((col) => {
                    const val = row[col];
                    const prevVal = prevRow[col];
                    const change = computeChange(val, prevVal);

                    return (
                      <td key={col} className="px-3 py-2">
                        <div className="flex items-center gap-1.5">
                          <span className="text-xs text-gray-200 font-mono">
                            {formatValue(val)}
                          </span>
                          {change && prevSnap && (
                            <ChangeIndicator change={change} />
                          )}
                        </div>
                      </td>
                    );
                  })}
                  <td className="px-2 py-2">
                    <button
                      onClick={() => onDelete(metricId, snap.id)}
                      className="w-5 h-5 rounded flex items-center justify-center text-gray-700 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-opacity"
                      aria-label={t('common.delete')}
                      title={t('common.delete')}
                    >
                      <Trash size={10} />
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// ── Multi-row table: each snapshot shown as a sub-table ──
function MultiRowTable({
  parsed,
  allParsed,
  pageOffset,
  columns,
  metricId,
  onDelete,
}: {
  parsed: { id: number; period_key: string; snapshot_at: string; rows: Record<string, unknown>[]; row_count?: number }[];
  allParsed: { id: number; period_key: string; snapshot_at: string; rows: Record<string, unknown>[]; row_count?: number }[];
  pageOffset: number;
  columns: string[];
  metricId: number;
  onDelete: (metricId: number, snapshotId: number) => void;
}) {
  const { t } = useTranslation();
  const [expandedId, setExpandedId] = useState<number | null>(parsed[0]?.id ?? null);

  return (
    <div className="space-y-2">
      {parsed.map((snap, idx) => {
        const isExpanded = expandedId === snap.id;
        const globalIdx = pageOffset + idx;
        const prevSnap = allParsed[globalIdx + 1];

        return (
          <div key={snap.id} className="rounded-lg border border-obsidian-700 overflow-hidden">
            {/* Snapshot header row */}
            <div
              onClick={() => setExpandedId(isExpanded ? null : snap.id)}
              className="flex items-center justify-between px-3 py-2 bg-obsidian-800/40 cursor-pointer hover:bg-obsidian-800/70 transition-colors"
            >
              <div className="flex items-center gap-3">
                <span className="text-xs text-gray-200 font-mono font-medium">{snap.period_key}</span>
                <span className="text-[10px] text-gray-500">{new Date(snap.snapshot_at).toLocaleString()}</span>
                <span className="text-[10px] px-1.5 py-0.5 rounded bg-obsidian-700 text-gray-400">
                  {snap.rows.length} 行
                </span>
                {prevSnap && (
                  <RowCountChange current={snap.rows.length} previous={prevSnap.rows.length} />
                )}
              </div>
              <button
                onClick={(e) => { e.stopPropagation(); onDelete(metricId, snap.id); }}
                className="w-5 h-5 rounded flex items-center justify-center text-gray-600 hover:text-red-400 transition-colors"
                aria-label={t('common.delete')}
                title={t('common.delete')}
              >
                <Trash size={10} />
              </button>
            </div>

            {/* Expanded data table */}
            {isExpanded && (
              <div className="overflow-x-auto">
                <table className="w-full text-left">
                  <thead>
                    <tr className="bg-obsidian-900/50">
                      <th className="px-3 py-1.5 text-[9px] text-gray-500 font-semibold uppercase tracking-wider border-b border-obsidian-700 w-8">
                        #
                      </th>
                      {columns.map((col) => (
                        <th key={col} className="px-3 py-1.5 text-[9px] text-gray-500 font-semibold uppercase tracking-wider border-b border-obsidian-700">
                          {col}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {snap.rows.map((row, rowIdx) => {
                      const prevRow = prevSnap?.rows[rowIdx];
                      return (
                        <tr key={rowIdx} className="border-b border-obsidian-800/50 hover:bg-obsidian-800/20">
                          <td className="px-3 py-1.5 text-[10px] text-gray-600">{rowIdx + 1}</td>
                          {columns.map((col) => {
                            const val = row[col];
                            const prevVal = prevRow?.[col];
                            const change = computeChange(val, prevVal);
                            return (
                              <td key={col} className="px-3 py-1.5">
                                <div className="flex items-center gap-1">
                                  <span className="text-[11px] text-gray-200 font-mono">{formatValue(val)}</span>
                                  {change && prevSnap && <ChangeIndicator change={change} />}
                                </div>
                              </td>
                            );
                          })}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

// ── Helpers ──

interface ChangeInfo {
  direction: 'up' | 'down' | 'same';
  diff: string;
  percent: string;
}

function computeChange(current: unknown, previous: unknown): ChangeInfo | null {
  const cur = parseFloat(String(current));
  const prev = parseFloat(String(previous));
  if (isNaN(cur) || isNaN(prev)) return null;
  if (cur === prev) return { direction: 'same', diff: '0', percent: '0%' };
  const diff = cur - prev;
  const percent = prev !== 0 ? ((diff / Math.abs(prev)) * 100).toFixed(1) + '%' : '-';
  return {
    direction: diff > 0 ? 'up' : 'down',
    diff: (diff > 0 ? '+' : '') + formatNum(diff),
    percent,
  };
}

function ChangeIndicator({ change }: { change: ChangeInfo }) {
  if (change.direction === 'same') {
    return (
      <span className="inline-flex items-center gap-0.5 text-[9px] text-gray-600">
        <Minus size={8} />
      </span>
    );
  }
  const isUp = change.direction === 'up';
  return (
    <span className={`inline-flex items-center gap-0.5 text-[9px] ${isUp ? 'text-green-400' : 'text-red-400'}`}>
      {isUp ? <ArrowUp size={8} weight="bold" /> : <ArrowDown size={8} weight="bold" />}
      <span>{change.percent}</span>
    </span>
  );
}

function RowCountChange({ current, previous }: { current: number; previous: number }) {
  if (current === previous) return null;
  const diff = current - previous;
  const isUp = diff > 0;
  return (
    <span className={`text-[9px] ${isUp ? 'text-green-400' : 'text-red-400'}`}>
      {isUp ? '+' : ''}{diff} 行
    </span>
  );
}

function formatValue(val: unknown): string {
  if (val === null || val === undefined) return '-';
  if (typeof val === 'number') return formatNum(val);
  if (typeof val === 'object') return JSON.stringify(val);
  return String(val);
}

function formatNum(n: number): string {
  if (Number.isInteger(n)) return n.toLocaleString();
  return n.toLocaleString(undefined, { maximumFractionDigits: 2 });
}

// ── Create Schedule Modal ──

function CreateScheduleModal({
  metrics,
  formMetricId,
  formType,
  formCron,
  formRetention,
  onMetricChange,
  onTypeChange,
  onCronChange,
  onRetentionChange,
  onSubmit,
  onClose,
  loading,
  t,
}: {
  metrics: MetricPool[];
  formMetricId: number | '' | undefined;
  formType: string;
  formCron: string;
  formRetention: string;
  onMetricChange: (v: number | '') => void;
  onTypeChange: (v: string) => void;
  onCronChange: (v: string) => void;
  onRetentionChange: (v: string) => void;
  onSubmit: () => void;
  onClose: () => void;
  loading: boolean;
  t: (key: string) => string;
}) {
  return (
    <>
      <div className="fixed inset-0 bg-black/60 z-40" onClick={onClose} />
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl w-full max-w-sm p-5" onClick={(e) => e.stopPropagation()}>
          <h3 className="text-xs font-semibold text-gray-100 mb-4">{t('snapshots.newSchedule')}</h3>

          <div className="space-y-3">
            <div>
              <label className="text-[10px] text-gray-500 block mb-1">{t('snapshots.selectMetric')}</label>
              <select
                value={formMetricId ?? ''}
                onChange={(e) => onMetricChange(e.target.value ? parseInt(e.target.value) : '')}
                className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50 transition-colors"
              >
                <option value="">{t('snapshots.selectMetric')}</option>
                {metrics.map((m) => (
                  <option key={m.id} value={m.id}>{m.name}</option>
                ))}
              </select>
            </div>

            <div>
              <label className="text-[10px] text-gray-500 block mb-1">{t('snapshots.scheduleType')}</label>
              <select
                value={formType}
                onChange={(e) => onTypeChange(e.target.value)}
                className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50 transition-colors"
              >
                <option value="hourly">{t('snapshots.hourly')}</option>
                <option value="daily">{t('snapshots.daily')}</option>
                <option value="weekly">{t('snapshots.weekly')}</option>
                <option value="monthly">{t('snapshots.monthly')}</option>
                <option value="cron">{t('snapshots.cron')}</option>
              </select>
            </div>

            {formType === 'cron' && (
              <div>
                <label className="text-[10px] text-gray-500 block mb-1">{t('snapshots.cronExpr')}</label>
                <input
                  type="text"
                  value={formCron}
                  onChange={(e) => onCronChange(e.target.value)}
                  placeholder={t('snapshots.cronPlaceholder')}
                  className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-colors"
                />
              </div>
            )}

            <div>
              <label className="text-[10px] text-gray-500 block mb-1">{t('snapshots.retentionDays')}</label>
              <input
                type="text"
                inputMode="numeric"
                value={formRetention}
                onChange={(e) => onRetentionChange(e.target.value.replace(/\D/g, ''))}
                placeholder={t('snapshots.retentionPlaceholder')}
                className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-2.5 py-1.5 text-[11px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-colors"
              />
            </div>
          </div>

          <div className="flex items-center justify-end gap-2 mt-5">
            <button
              onClick={onClose}
              className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
            >
              {t('common.cancel')}
            </button>
            <button
              onClick={onSubmit}
              disabled={!formMetricId || loading}
              className="text-xs text-[#08080c] font-medium bg-amber-500 hover:bg-amber-400 px-4 py-1.5 rounded-md disabled:opacity-50 transition-premium"
            >
              {t('common.save')}
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
