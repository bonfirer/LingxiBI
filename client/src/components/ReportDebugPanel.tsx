import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { X, ArrowClockwise, Warning, CheckCircle } from '@phosphor-icons/react';
import { reportsApi, type ReportDebugEntry } from '../lib/api';

interface Props {
  reportId: number;
  onClose: () => void;
}

/** Bottom debug panel: shows the SQL each report dataset runs (base +
 *  filter-wrapped), bound params, timing, row counts and errors. Only mounted
 *  while open, so no SQL debug trace is produced when the panel is closed. */
export function ReportDebugPanel({ reportId, onClose }: Props) {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<ReportDebugEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Bumped by the manual refresh button to re-trigger the fetch effect.
  const [nonce, setNonce] = useState(0);

  // Fetch the trace on open and whenever the report or refresh nonce changes.
  // State is only updated after the await (guarded by `cancelled`), so the
  // effect body performs no synchronous setState.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const data = await reportsApi.debug(reportId);
        if (!cancelled) {
          setEntries(data);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [reportId, nonce]);

  const reload = () => {
    setLoading(true);
    setNonce((n) => n + 1);
  };

  return (
    <div className="flex-shrink-0 border-t border-obsidian-700 bg-obsidian-950 max-h-[38vh] flex flex-col">
      <div className="flex items-center gap-2 px-4 py-2 border-b border-obsidian-800 bg-obsidian-900">
        <span className="text-[11px] font-semibold text-amber-500/90 font-mono">DEBUG · SQL</span>
        <span className="text-[10px] text-gray-600">{t('reportDetail.debug.subtitle')}</span>
        <div className="flex-1" />
        <button
          onClick={reload}
          disabled={loading}
          className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 disabled:opacity-50 transition-premium"
          title={t('common.refresh')}
        >
          <ArrowClockwise size={11} className={loading ? 'animate-spin' : ''} /> {t('common.refresh')}
        </button>
        <button onClick={onClose} className="text-gray-500 hover:text-gray-300"><X size={14} /></button>
      </div>

      <div className="overflow-y-auto scrollbar-thin p-3 space-y-2 font-mono">
        {error && <p className="text-[11px] text-red-400">{error}</p>}
        {!error && !loading && entries.length === 0 && (
          <p className="text-[11px] text-gray-600">{t('reportDetail.debug.noDatasets')}</p>
        )}

        {entries.map((e) => (
          <div key={e.dataset_id} className="rounded-lg border border-obsidian-800 bg-obsidian-900/60 overflow-hidden">
            <div className="flex items-center gap-2 px-3 py-1.5 bg-obsidian-800/60 border-b border-obsidian-800">
              {e.error ? (
                <Warning size={12} className="text-red-400 flex-shrink-0" weight="fill" />
              ) : (
                <CheckCircle size={12} className="text-data-green flex-shrink-0" weight="fill" />
              )}
              <span className="text-[11px] text-gray-200 truncate">{e.name}</span>
              <span className="text-[9px] text-gray-600 uppercase">{e.db_type}</span>
              <div className="flex-1" />
              {e.filter_count > 0 && (
                <span className="text-[9px] text-amber-500/80">{t('reportDetail.debug.filters', { count: e.filter_count })}</span>
              )}
              {e.row_count != null && (
                <span className="text-[9px] text-gray-500">{e.row_count} rows</span>
              )}
              <span className="text-[9px] text-gray-500">{e.duration_ms} ms</span>
              <span className="text-[9px] text-gray-600" title={e.executed_at}>{new Date(e.executed_at).toLocaleTimeString()}</span>
            </div>

            <div className="p-2.5 space-y-1.5">
              <div>
                <span className="text-[9px] text-gray-600 uppercase">{t('reportDetail.debug.executedSql')}</span>
                <pre className="mt-0.5 text-[10.5px] text-gray-300 whitespace-pre-wrap break-all bg-obsidian-950 rounded p-2 border border-obsidian-800">{e.effective_sql}</pre>
              </div>

              {e.params.length > 0 && (
                <div>
                  <span className="text-[9px] text-gray-600 uppercase">{t('reportDetail.debug.params')}</span>
                  <pre className="mt-0.5 text-[10.5px] text-cyan-300/90 whitespace-pre-wrap break-all bg-obsidian-950 rounded p-2 border border-obsidian-800">{JSON.stringify(e.params)}</pre>
                </div>
              )}

              {e.filter_count > 0 && e.effective_sql !== e.base_sql && (
                <details>
                  <summary className="text-[9px] text-gray-600 uppercase cursor-pointer hover:text-gray-400">{t('reportDetail.debug.baseSql')}</summary>
                  <pre className="mt-0.5 text-[10.5px] text-gray-500 whitespace-pre-wrap break-all bg-obsidian-950 rounded p-2 border border-obsidian-800">{e.base_sql}</pre>
                </details>
              )}

              {e.error && (
                <pre className="text-[10.5px] text-red-400 whitespace-pre-wrap break-all bg-red-950/20 rounded p-2 border border-red-900/40">{e.error}</pre>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
