import { useEffect, useState, useCallback, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { ChartBar, Trash, ArrowRight, Clock, ChatCircle, Plus, X, CaretLeft, CaretRight } from '@phosphor-icons/react';
import { reportsApi } from '../lib/api';
import type { Report } from '../lib/types';
import { PageHeader, ErrorBanner, EmptyState, ConfirmDialog } from '../components/ui';
import { fetchEmbedToken } from '../lib/embedToken';
import { toast } from '../stores/toastStore';

export default function ReportsPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [reports, setReports] = useState<Report[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Report | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [creating, setCreating] = useState(false);
  const [page, setPage] = useState(1);

  const PAGE_SIZE = 6;
  const totalPages = Math.max(1, Math.ceil(reports.length / PAGE_SIZE));
  // Clamp the current page if the list shrinks (e.g. after a delete).
  const currentPage = Math.min(page, totalPages);
  const pageStart = (currentPage - 1) * PAGE_SIZE;
  const pagedReports = reports.slice(pageStart, pageStart + PAGE_SIZE);

  const fetchReports = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);
      const data = await reportsApi.list();
      setReports(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    fetchReports();
  }, [fetchReports]);

  // Poll while any report is generating, to update status live
  useEffect(() => {
    const anyGenerating = reports.some((r) => r.generation_status === 'generating');
    if (!anyGenerating) return;
    const timer = setInterval(() => {
      reportsApi.list().then(setReports).catch(() => {});
    }, 4000);
    return () => clearInterval(timer);
  }, [reports]);

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    try {
      await reportsApi.delete(deleteTarget.id);
      await fetchReports();
      window.dispatchEvent(new Event('reports-updated'));
      toast.success(t('reports.deleted'));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('errors.deleteFailed'));
    } finally {
      setDeleteTarget(null);
    }
  };

  const handleCreate = async () => {
    if (!newTitle.trim() || creating) return;
    setCreating(true);
    try {
      const r = await reportsApi.create({ title: newTitle.trim(), pool_ids: [] });
      window.dispatchEvent(new Event('reports-updated'));
      setShowCreate(false);
      setNewTitle('');
      navigate(`/reports/${r.id}`);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t('errors.saveFailed'));
    } finally {
      setCreating(false);
    }
  };

  const timeAgo = (dateStr?: string) => {
    if (!dateStr) return '';
    const diff = Date.now() - new Date(dateStr).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return t('time.justNow');
    if (mins < 60) return t('time.minutesAgo', { n: mins });
    const hours = Math.floor(mins / 60);
    if (hours < 24) return t('time.hoursAgo', { n: hours });
    return t('time.daysAgo', { n: Math.floor(hours / 24) });
  };

  return (
    <div className="p-6">
      <PageHeader
        title={t('reports.title')}
        description={t('reports.description')}
        action={
          <div className="flex items-center gap-2">
            <button
              onClick={() => navigate('/conversations')}
              className="flex items-center gap-1.5 text-gray-300 hover:text-gray-100 border border-obsidian-700 hover:border-obsidian-600 text-xs px-3 py-2 rounded-lg transition-premium"
            >
              <ChatCircle size={14} />
              {t('reports.startConversation')}
            </button>
            <button
              onClick={() => { setNewTitle(''); setShowCreate(true); }}
              className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-3.5 py-2 rounded-lg transition-premium active:translate-y-[1px] shadow-[0_4px_16px_-6px_rgba(245,158,11,0.5)]"
            >
              <Plus size={15} weight="bold" />
              {t('reports.createReport')}
            </button>
          </div>
        }
      />

      {/* Error */}
      {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

      {/* Loading */}
      {loading && (
        <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
          {[1, 2].map((i) => (
            <div
              key={i}
              className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-4 animate-pulse"
            >
              <div className="space-y-3">
                <div className="h-4 bg-obsidian-700 rounded w-48" />
                <div className="h-3 bg-obsidian-700 rounded w-32" />
                <div className="grid grid-cols-3 gap-3">
                  {[1, 2, 3].map((j) => (
                    <div key={j} className="h-16 bg-obsidian-700 rounded-lg" />
                  ))}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Empty State */}
      {!loading && reports.length === 0 && (
        <EmptyState
          icon={ChartBar}
          title={t('reports.empty.title')}
          description={t('reports.empty.description')}
          action={
            <div className="flex items-center gap-2">
              <button
                onClick={() => { setNewTitle(''); setShowCreate(true); }}
                className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]"
              >
                <Plus size={15} weight="bold" />
                {t('reports.createReport')}
              </button>
              <button
                onClick={() => navigate('/conversations')}
                className="flex items-center gap-1.5 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-200 text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]"
              >
                <ChatCircle size={14} />
                {t('reports.startConversation')}
              </button>
            </div>
          }
        />
      )}

      {/* Report Cards */}
      {!loading && reports.length > 0 && (
        <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
          {pagedReports.map((report, i) => {
            const visCount = report.config?.visualizations?.length || 0;
            const visTypes = report.config?.visualizations?.map((v) => v.type) || [];
            return (
              <div
                key={report.id}
                onClick={() => navigate(`/reports/${report.id}`)}
                className="bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden hover:border-obsidian-600 transition-premium group stagger-item cursor-pointer"
                style={{ animationDelay: `${i * 80}ms` }}
              >
                <div className="p-4">
                  <div className="flex items-start justify-between mb-3">
                    <div className="min-w-0 flex-1">
                      <h3 className="text-sm font-semibold text-gray-200 truncate group-hover:text-amber-500 transition-premium flex items-center gap-2">
                        <ReportCardStatusDot report={report} />
                        {report.title}
                      </h3>
                      <p className="text-[10px] text-gray-500 mt-1">
                        {visCount > 0
                          ? t('reports.visualization', { count: visCount })
                          : report.html_content
                            ? t('reports.aiRendered')
                            : t('reports.noVisualization')}
                        {report.description && ` · ${report.description}`}
                      </p>
                    </div>
                    <div className="flex items-center gap-1 text-[10px] text-gray-600 ml-2 flex-shrink-0">
                      <Clock size={10} />
                      {timeAgo(report.created_at)}
                    </div>
                  </div>

                  {/* Visualization type badges + pool chips */}
                  <div className="flex gap-1.5 flex-wrap mb-3">
                    {visTypes.slice(0, 4).map((type, idx) => (
                      <span
                        key={idx}
                        className="bg-obsidian-800 text-gray-400 text-[9px] px-1.5 py-0.5 rounded font-mono uppercase"
                      >
                        {type}
                      </span>
                    ))}
                    {report.pool_ids?.slice(0, 3).map((pid) => (
                      <span
                        key={pid}
                        className="bg-amber-500/10 text-amber-500/80 text-[9px] px-1.5 py-0.5 rounded-full font-mono"
                      >
                        pool#{pid}
                      </span>
                    ))}
                  </div>

                  {/* Report preview based on status */}
                  <div className="rounded-lg h-[120px] relative overflow-hidden">
                    {report.generation_status === 'generating' ? (
                      // Generating — skeleton animation
                      <div className="h-full bg-obsidian-800 p-2 space-y-1.5">
                        <div className="flex gap-1.5">
                          {[1,2,3].map(i => <div key={i} className="flex-1 h-5 rounded bg-obsidian-700 shimmer-surface" />)}
                        </div>
                        <div className="h-12 rounded bg-obsidian-700 shimmer-surface" />
                        <div className="flex gap-1.5">
                          <div className="flex-1 h-6 rounded bg-obsidian-700 shimmer-surface" />
                          <div className="flex-1 h-6 rounded bg-obsidian-700 shimmer-surface" />
                        </div>
                      </div>
                    ) : report.html_content ? (
                      // Has HTML — lazy-mount a static preview (no live data fetching)
                      <LazyReportPreview report={report} />
                    ) : report.generation_status === 'failed' ? (
                      // Failed
                      <div className="h-full bg-obsidian-800 flex items-center justify-center">
                        <span className="text-[10px] text-red-400/60">⚠ 生成失败</span>
                      </div>
                    ) : (
                      // Empty — no content yet
                      <div className="h-full bg-obsidian-800 flex flex-col items-center justify-center">
                        <ChartBar size={20} className="text-gray-700 mb-1" />
                        <span className="text-[9px] text-gray-600">等待生成</span>
                      </div>
                    )}
                  </div>
                </div>

                <div className="flex items-center gap-1 px-4 py-2 border-t border-obsidian-700 bg-obsidian-800/50">
                  <button
                    onClick={(e) => { e.stopPropagation(); setDeleteTarget(report); }}
                    className="flex items-center gap-1 text-[10px] text-gray-600 hover:text-red-400 px-2 py-1 rounded transition-premium"
                  >
                    <Trash size={12} /> {t('common.delete')}
                  </button>
                  <span className="ml-auto flex items-center gap-1 text-[10px] text-amber-500 group-hover:text-amber-400 px-2 py-1 rounded transition-premium font-medium">
                    {t('common.open')} <ArrowRight size={10} />
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Pagination */}
      {!loading && reports.length > PAGE_SIZE && (
        <div className="flex items-center justify-between mt-5">
          <span className="text-[11px] text-gray-500">
            {t('reports.pagination.summary', {
              from: pageStart + 1,
              to: Math.min(pageStart + PAGE_SIZE, reports.length),
              total: reports.length,
            })}
          </span>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setPage(currentPage - 1)}
              disabled={currentPage <= 1}
              className="flex items-center gap-1 text-[11px] text-gray-300 hover:text-gray-100 border border-obsidian-700 hover:border-obsidian-600 px-2.5 py-1.5 rounded-lg transition-premium disabled:opacity-40 disabled:cursor-not-allowed"
            >
              <CaretLeft size={12} />
              {t('reports.pagination.prev')}
            </button>
            <span className="text-[11px] text-gray-400 tabular-nums px-1">
              {t('reports.pagination.page', { page: currentPage, pages: totalPages })}
            </span>
            <button
              onClick={() => setPage(currentPage + 1)}
              disabled={currentPage >= totalPages}
              className="flex items-center gap-1 text-[11px] text-gray-300 hover:text-gray-100 border border-obsidian-700 hover:border-obsidian-600 px-2.5 py-1.5 rounded-lg transition-premium disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {t('reports.pagination.next')}
              <CaretRight size={12} />
            </button>
          </div>
        </div>
      )}

      {/* Create Report Modal */}
      {showCreate && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setShowCreate(false)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-[400px] shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between mb-3">
              <h3 className="text-sm font-semibold text-gray-100 flex items-center gap-2">
                <ChartBar size={16} className="text-amber-500" />
                {t('reports.createTitle')}
              </h3>
              <button onClick={() => setShowCreate(false)} className="w-7 h-7 rounded-md flex items-center justify-center text-gray-500 hover:text-gray-300 hover:bg-obsidian-700 transition-premium"><X size={16} /></button>
            </div>
            <p className="text-[11px] text-gray-500 mb-3">{t('reports.createHint')}</p>
            <label className="block text-[11px] font-medium text-gray-400 mb-1">{t('reports.reportName')}</label>
            <input
              value={newTitle}
              onChange={(e) => setNewTitle(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreate(); if (e.key === 'Escape') setShowCreate(false); }}
              placeholder={t('reports.reportNamePlaceholder')}
              autoFocus
              className="w-full bg-obsidian-800 border border-obsidian-700 rounded-md px-2.5 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
            />
            <div className="flex items-center justify-end gap-2 mt-5">
              <button onClick={() => setShowCreate(false)} className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium">
                {t('common.cancel')}
              </button>
              <button
                onClick={handleCreate}
                disabled={!newTitle.trim() || creating}
                className="text-xs text-[#08080c] bg-amber-500 hover:bg-amber-400 px-4 py-1.5 rounded-md font-semibold disabled:opacity-50 transition-premium"
              >
                {creating ? t('common.loading') : t('reports.createAndOpen')}
              </button>
            </div>
          </div>
        </div>
      )}

      <ConfirmDialog
        open={!!deleteTarget}
        title={t('reports.deleteConfirm.title')}
        message={t('reports.deleteConfirm.message', { name: deleteTarget?.title })}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        onConfirm={confirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />
    </div>
  );
}

// Status dot for a report card
function ReportCardStatusDot({ report }: { report: Report }) {
  const status = report.generation_status;
  if (status === 'generating') {
    return <span className="w-2 h-2 rounded-full bg-amber-500 dot-breathe flex-shrink-0" title="生成中" />;
  }
  if (status === 'failed') {
    return <span className="w-2 h-2 rounded-full bg-red-500 flex-shrink-0" title="生成失败" />;
  }
  if (report.html_content || status === 'done') {
    return <span className="w-2 h-2 rounded-full bg-data-green flex-shrink-0" title="已生成" />;
  }
  return <span className="w-2 h-2 rounded-full bg-gray-600 flex-shrink-0" title="未生成" />;
}

// Global gate limiting how many preview iframes load at once. Loading every
// visible card's iframe simultaneously saturates the browser's per-origin
// connection cap, and a single slow response could leave the rest of the
// thumbnails spinning forever. We cap concurrency and hand slots out in order.
const PREVIEW_MAX_CONCURRENT = 3;
// Safety net: never let one stalled iframe hold its slot indefinitely.
const PREVIEW_LOAD_TIMEOUT_MS = 15000;

let previewActive = 0;
const previewQueue: (() => void)[] = [];

// Run `task` when a slot is free (immediately, or once one is released).
// Returns a `cancel` fn that removes the task if it hasn't started yet.
function schedulePreview(task: () => void): () => void {
  if (previewActive < PREVIEW_MAX_CONCURRENT) {
    previewActive++;
    task();
    return () => {};
  }
  previewQueue.push(task);
  return () => {
    const i = previewQueue.indexOf(task);
    if (i >= 0) previewQueue.splice(i, 1);
  };
}

// Free a held slot: hand it directly to the next waiter, or decrement.
function releasePreviewSlot() {
  const next = previewQueue.shift();
  if (next) {
    next();
  } else if (previewActive > 0) {
    previewActive--;
  }
}

// Lazy-mounted static preview: the iframe is only created once the card scrolls
// into view, and loads in preview mode (?preview=1) so no live SQL queries run.
function LazyReportPreview({ report }: { report: Report }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const releaseRef = useRef<(() => void) | null>(null);
  const [visible, setVisible] = useState(false);
  const [src, setSrc] = useState<string | null>(null);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setVisible(true);
          observer.disconnect();
        }
      },
      { rootMargin: '200px' }
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Once visible, wait for a free concurrency slot, then mint a short-lived
  // embed token (cached/shared across cards) rather than putting the long-lived
  // session JWT in the iframe URL.
  useEffect(() => {
    if (!visible) return;
    let disposed = false;
    let started = false; // whether our task ran (i.e. we hold a slot)
    let released = false;
    let timeout: ReturnType<typeof setTimeout> | undefined;

    const release = () => {
      if (released) return;
      released = true;
      if (timeout) clearTimeout(timeout);
      releasePreviewSlot();
    };
    releaseRef.current = release;

    const cancel = schedulePreview(() => {
      started = true;
      if (disposed) {
        release();
        return;
      }
      fetchEmbedToken().then((embed) => {
        if (disposed) {
          release();
          return;
        }
        const token = embed || localStorage.getItem('token') || '';
        setSrc(
          `/api/reports/${report.id}/html?preview=1&token=${encodeURIComponent(token)}&t=${report.updated_at || ''}`
        );
        // Release the slot even if onLoad never fires (stalled/failed iframe).
        timeout = setTimeout(release, PREVIEW_LOAD_TIMEOUT_MS);
      });
    });

    return () => {
      disposed = true;
      if (started) release();
      else cancel();
    };
  }, [visible, report.id, report.updated_at]);

  return (
    <div ref={containerRef} className="w-full h-full bg-obsidian-800">
      {src ? (
        <iframe
          src={src}
          className="w-full h-full border-0 pointer-events-none"
          style={{ transform: 'scale(0.35)', transformOrigin: 'top left', width: '286%', height: '286%' }}
          tabIndex={-1}
          loading="lazy"
          // Static thumbnail of AI-generated HTML: allow scripts (charts need
          // them) but block top-navigation, form submission, popups, downloads,
          // same-origin access, etc. Preview-only — no user interaction.
          sandbox="allow-scripts"
          onLoad={() => releaseRef.current?.()}
          onError={() => releaseRef.current?.()}
        />
      ) : (
        <div className="h-full flex items-center justify-center">
          <ChartBar size={20} className="text-gray-700 animate-pulse" />
        </div>
      )}
    </div>
  );
}
