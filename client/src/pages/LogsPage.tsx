import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { ClockCounterClockwise, ArrowClockwise, CaretLeft, CaretRight } from '@phosphor-icons/react';
import { PageHeader } from '../components/ui';

interface LogEntry {
  id: number;
  request_type: string;
  model: string | null;
  duration_ms: number | null;
  status: string | null;
  error_message: string | null;
  context: string | null;
  input_params: string | null;
  output_result: string | null;
  created_at: string | null;
}

interface PaginatedResponse {
  data: LogEntry[];
  total: number;
  page: number;
  page_size: number;
  total_pages: number;
}

const PAGE_SIZE = 20;

export default function LogsPage() {
  const { t } = useTranslation();
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [totalPages, setTotalPages] = useState(1);

  const fetchLogs = useCallback((p: number = page) => {
    setLoading(true);
    fetch(`/api/ai-logs?page=${p}&page_size=${PAGE_SIZE}`, {
      headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
    })
      .then((r) => r.json())
      .then((res: PaginatedResponse) => {
        setLogs(res.data);
        setTotal(res.total);
        setTotalPages(res.total_pages);
        setPage(res.page);
      })
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [page]);

  useEffect(() => { fetchLogs(1); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const goToPage = (p: number) => {
    if (p < 1 || p > totalPages) return;
    setExpandedId(null);
    fetchLogs(p);
  };

  const formatDuration = (ms: number | null) => {
    if (!ms) return '—';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  };

  const formatTime = (ts: string | null) => {
    if (!ts) return '';
    return new Date(ts).toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit' });
  };

  return (
    <div className="p-4 md:p-6">
      <PageHeader
        title={t('logs.title')}
        description={t('logs.description')}
        action={
          <button onClick={() => fetchLogs(page)} className="flex items-center gap-1.5 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-300 text-xs px-3 py-2 rounded-lg transition-premium">
            <ArrowClockwise size={14} /> {t('common.refresh')}
          </button>
        }
      />

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center">
          <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
          <span className="text-xs text-gray-500">{t('common.loading')}</span>
        </div>
      ) : logs.length === 0 && page === 1 ? (
        <div className="text-center py-16">
          <ClockCounterClockwise size={32} className="text-gray-700 mx-auto mb-3" />
          <p className="text-sm text-gray-500">{t('logs.empty')}</p>
        </div>
      ) : (
        <>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden mt-4">
            <table className="w-full text-[11px]">
              <thead>
                <tr className="border-b border-obsidian-700 bg-obsidian-800/50">
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.time')}</th>
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.type')}</th>
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.model')}</th>
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.duration')}</th>
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.status')}</th>
                  <th className="px-4 py-2.5 text-left text-gray-500 font-medium">{t('logs.context')}</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((log) => (
                  <LogTableRow
                    key={log.id}
                    log={log}
                    expanded={expandedId === log.id}
                    onToggle={() => setExpandedId(expandedId === log.id ? null : log.id)}
                    formatTime={formatTime}
                    formatDuration={formatDuration}
                    t={t}
                  />
                ))}
              </tbody>
            </table>
          </div>

          {/* Pagination */}
          <div className="flex items-center justify-between mt-4">
            <span className="text-[11px] text-gray-500">
              {t('logs.totalRecords', { total })}
            </span>
            <div className="flex items-center gap-1">
              <button
                onClick={() => goToPage(page - 1)}
                disabled={page <= 1}
                className="flex items-center justify-center w-7 h-7 rounded-md border border-obsidian-700 text-gray-400 hover:text-gray-200 hover:border-obsidian-600 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
              >
                <CaretLeft size={12} />
              </button>
              {generatePageNumbers(page, totalPages).map((p, i) =>
                p === '...' ? (
                  <span key={`ellipsis-${i}`} className="w-7 h-7 flex items-center justify-center text-[10px] text-gray-600">…</span>
                ) : (
                  <button
                    key={p}
                    onClick={() => goToPage(p as number)}
                    className={`w-7 h-7 rounded-md text-[11px] font-medium transition-premium ${
                      p === page
                        ? 'bg-amber-500/10 border border-amber-500/30 text-amber-500'
                        : 'border border-obsidian-700 text-gray-400 hover:text-gray-200 hover:border-obsidian-600'
                    }`}
                  >
                    {p}
                  </button>
                )
              )}
              <button
                onClick={() => goToPage(page + 1)}
                disabled={page >= totalPages}
                className="flex items-center justify-center w-7 h-7 rounded-md border border-obsidian-700 text-gray-400 hover:text-gray-200 hover:border-obsidian-600 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
              >
                <CaretRight size={12} />
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/** Generate page number array with ellipsis for large page counts */
function generatePageNumbers(current: number, total: number): (number | string)[] {
  if (total <= 7) return Array.from({ length: total }, (_, i) => i + 1);
  const pages: (number | string)[] = [];
  pages.push(1);
  if (current > 3) pages.push('...');
  const start = Math.max(2, current - 1);
  const end = Math.min(total - 1, current + 1);
  for (let i = start; i <= end; i++) pages.push(i);
  if (current < total - 2) pages.push('...');
  pages.push(total);
  return pages;
}

function LogTableRow({ log, expanded, onToggle, formatTime, formatDuration, t }: {
  log: LogEntry;
  expanded: boolean;
  onToggle: () => void;
  formatTime: (t: string | null) => string;
  formatDuration: (ms: number | null) => string;
  t: (k: string) => string;
}) {
  const [detail, setDetail] = useState<LogEntry | null>(null);
  const [loadingDetail, setLoadingDetail] = useState(false);

  const handleToggle = async () => {
    if (!expanded && !detail) {
      setLoadingDetail(true);
      try {
        const res = await fetch(`/api/ai-logs/${log.id}`, {
          headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
        });
        if (res.ok) setDetail(await res.json());
      } catch {}
      setLoadingDetail(false);
    }
    onToggle();
  };

  const displayLog = detail || log;

  return (
    <>
      <tr onClick={handleToggle} className="border-b border-obsidian-700/30 hover:bg-obsidian-800/30 cursor-pointer">
        <td className="px-4 py-2.5 text-gray-500 whitespace-nowrap">{formatTime(log.created_at)}</td>
        <td className="px-4 py-2.5">
          <span className={`px-2 py-0.5 rounded text-[10px] font-mono ${
            log.request_type === 'chat' ? 'bg-blue-400/10 text-blue-400' :
            log.request_type === 'html_generation' ? 'bg-amber-500/10 text-amber-500' :
            'bg-purple-400/10 text-purple-400'
          }`}>
            {log.request_type}
          </span>
        </td>
        <td className="px-4 py-2.5 text-gray-400 font-mono text-[10px]">{log.model || '—'}</td>
        <td className="px-4 py-2.5 text-gray-300 font-mono">{formatDuration(log.duration_ms)}</td>
        <td className="px-4 py-2.5">
          <span className={`text-[10px] ${log.status === 'success' ? 'text-data-green' : 'text-red-400'}`}>
            {log.status === 'success' ? '✓' : '✗'} {log.status}
          </span>
        </td>
        <td className="px-4 py-2.5 text-gray-500 max-w-[250px] truncate">
          {log.error_message || log.context || '—'}
        </td>
      </tr>
      {expanded && (
        <tr className="border-b border-obsidian-700/30">
          <td colSpan={6} className="px-4 py-4 bg-obsidian-800/20">
            {loadingDetail ? (
              <div className="flex items-center gap-2 py-4 justify-center">
                <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
                <span className="text-xs text-gray-500">{t('common.loading')}</span>
              </div>
            ) : (
              <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
                {displayLog.input_params && (
                  <div>
                    <span className="text-[10px] text-amber-500 font-semibold uppercase block mb-1.5">{t('logs.input')}</span>
                    <pre className="text-[10px] text-gray-400 font-mono whitespace-pre-wrap break-all bg-obsidian-900 rounded-lg p-3 max-h-[400px] overflow-y-auto scrollbar-thin border border-obsidian-700">
                      {displayLog.input_params}
                    </pre>
                  </div>
                )}
                {displayLog.output_result && (
                  <div>
                    <span className="text-[10px] text-data-green font-semibold uppercase block mb-1.5">{t('logs.output')}</span>
                    <pre className="text-[10px] text-gray-400 font-mono whitespace-pre-wrap break-all bg-obsidian-900 rounded-lg p-3 max-h-[400px] overflow-y-auto scrollbar-thin border border-obsidian-700">
                      {displayLog.output_result}
                    </pre>
                  </div>
                )}
                {displayLog.error_message && (
                  <div className="lg:col-span-2">
                    <span className="text-[10px] text-red-400 font-semibold uppercase block mb-1.5">{t('logs.error')}</span>
                    <pre className="text-[10px] text-red-400/80 font-mono whitespace-pre-wrap break-all bg-red-500/5 rounded-lg p-3 border border-red-500/10">
                      {displayLog.error_message}
                    </pre>
                  </div>
                )}
                {!displayLog.input_params && !displayLog.output_result && !displayLog.error_message && (
                  <p className="text-[10px] text-gray-600 italic">{t('logs.noDetail')}</p>
                )}
              </div>
            )}
          </td>
        </tr>
      )}
    </>
  );
}
