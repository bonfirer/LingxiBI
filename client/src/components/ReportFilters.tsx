import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Trash, Check, X, Funnel } from '@phosphor-icons/react';
import {
  reportsApi,
  type Report,
  type ReportDataSource,
  type ReportFilter,
  type ReportFilterTarget,
  type FilterCondition,
} from '../lib/api';

type Op = FilterCondition['op'];
const OPS: Op[] = ['=', '!=', '>', '>=', '<', '<=', 'LIKE', 'IN', 'BETWEEN'];

interface Draft {
  key: string;
  label: string;
  op: Op;
  value: string;
  from: string;
  to: string;
  targets: ReportFilterTarget[];
}

function coerce(raw: string): string | number {
  const s = raw.trim();
  if (s !== '' && !Number.isNaN(Number(s))) return Number(s);
  return s;
}

function draftToFilter(d: Draft): ReportFilter {
  let value: unknown;
  if (d.op === 'IN') {
    value = d.value.split(',').map((v) => coerce(v)).filter((v) => v !== '');
  } else if (d.op === 'BETWEEN') {
    value = [coerce(d.from), coerce(d.to)];
  } else {
    value = d.value.trim() === '' ? '' : coerce(d.value);
  }
  return {
    key: d.key,
    label: d.label.trim() || d.key,
    op: d.op,
    value,
    targets: d.targets.filter((t) => t.datasource_id && t.column.trim()),
  };
}

function filterToDraft(f: ReportFilter): Draft {
  const base: Draft = {
    key: f.key,
    label: f.label ?? '',
    op: f.op,
    value: '',
    from: '',
    to: '',
    targets: f.targets ?? [],
  };
  if (f.op === 'IN' && Array.isArray(f.value)) {
    base.value = (f.value as unknown[]).join(', ');
  } else if (f.op === 'BETWEEN' && Array.isArray(f.value)) {
    base.from = String((f.value as unknown[])[0] ?? '');
    base.to = String((f.value as unknown[])[1] ?? '');
  } else {
    base.value = f.value == null ? '' : String(f.value);
  }
  return base;
}

/** Columns available for a given report datasource, from its cached result. */
function columnsOf(ds?: ReportDataSource): string[] {
  const rows = ds?.result_cache;
  if (Array.isArray(rows) && rows.length > 0 && typeof rows[0] === 'object' && rows[0]) {
    return Object.keys(rows[0] as Record<string, unknown>);
  }
  return [];
}

interface Props {
  report: Report;
  datasources: ReportDataSource[];
  onClose: () => void;
  onApplied: (report: Report) => void;
}

/** Report-level global filter editor. Each control maps a value to one or more
 *  dataset columns; applying re-executes the affected datasets server-side. */
export function ReportFilters({ report, datasources, onClose, onApplied }: Props) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Draft[]>(() => (report.report_filters ?? []).map(filterToDraft));

  const update = (i: number, patch: Partial<Draft>) =>
    setDrafts((prev) => prev.map((d, idx) => (idx === i ? { ...d, ...patch } : d)));

  const addFilter = () =>
    setDrafts((prev) => [
      ...prev,
      {
        key: `f_${Date.now()}_${prev.length}`,
        label: '',
        op: '=',
        value: '',
        from: '',
        to: '',
        targets: datasources.length > 0 ? [{ datasource_id: datasources[0].datasource_id, column: '' }] : [],
      },
    ]);

  const removeFilter = (i: number) => setDrafts((prev) => prev.filter((_, idx) => idx !== i));

  const addTarget = (i: number) =>
    update(i, {
      targets: [
        ...drafts[i].targets,
        { datasource_id: datasources[0]?.datasource_id ?? 0, column: '' },
      ],
    });

  const updateTarget = (i: number, ti: number, patch: Partial<ReportFilterTarget>) =>
    update(i, { targets: drafts[i].targets.map((tg, idx) => (idx === ti ? { ...tg, ...patch } : tg)) });

  const removeTarget = (i: number, ti: number) =>
    update(i, { targets: drafts[i].targets.filter((_, idx) => idx !== ti) });

  const apply = async () => {
    setSaving(true);
    setError(null);
    try {
      const filters = drafts.map(draftToFilter).filter((f) => f.targets.length > 0);
      const updated = await reportsApi.setReportFilters(report.id, filters);
      onApplied(updated);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const inputCls =
    'bg-obsidian-900 border border-obsidian-700 rounded px-1.5 py-1 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50';

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="bg-obsidian-900 border border-obsidian-700 rounded-2xl w-[540px] max-h-[80vh] overflow-hidden shadow-2xl shadow-black/50"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-4 border-b border-obsidian-700 bg-obsidian-800/30">
          <h2 className="text-sm font-semibold text-gray-200 flex items-center gap-2">
            <Funnel size={16} className="text-amber-500" weight="fill" />
            {t('reportDetail.globalFilters.title')}
          </h2>
          <button onClick={onClose} className="w-7 h-7 rounded-md flex items-center justify-center text-gray-500 hover:text-gray-300 hover:bg-obsidian-700 transition-premium">
            <X size={16} />
          </button>
        </div>

        <div className="p-5 overflow-y-auto max-h-[60vh] scrollbar-thin space-y-4">
          <p className="text-[11px] text-gray-400 leading-relaxed bg-obsidian-800/50 border border-obsidian-700/50 rounded-lg px-3 py-2.5">
            {t('reportDetail.globalFilters.hint')}
          </p>
          {drafts.length === 0 && (
            <div className="flex flex-col items-center justify-center py-8 text-center">
              <Funnel size={32} className="text-gray-700 mb-2" />
              <p className="text-[11px] text-gray-500">{t('reportDetail.globalFilters.empty')}</p>
            </div>
          )}

          {drafts.map((d, i) => {
            const targetCols = (dsId: number) => columnsOf(datasources.find((x) => x.datasource_id === dsId));
            return (
              <div key={d.key} className="p-3 rounded-xl bg-obsidian-800/50 border border-obsidian-700 space-y-3 hover:border-obsidian-600 transition-premium">
                <div className="flex items-center gap-2">
                  <input
                    value={d.label}
                    onChange={(e) => update(i, { label: e.target.value })}
                    placeholder={t('reportDetail.globalFilters.label')}
                    className={`${inputCls} w-32`}
                  />
                  <select value={d.op} onChange={(e) => update(i, { op: e.target.value as Op })} className={inputCls}>
                    {OPS.map((op) => <option key={op} value={op}>{op}</option>)}
                  </select>
                  {d.op === 'BETWEEN' ? (
                    <>
                      <input value={d.from} onChange={(e) => update(i, { from: e.target.value })} placeholder={t('reportDetail.filters.betweenFrom')} className={`${inputCls} w-24`} />
                      <input value={d.to} onChange={(e) => update(i, { to: e.target.value })} placeholder={t('reportDetail.filters.betweenTo')} className={`${inputCls} w-24`} />
                    </>
                  ) : (
                    <input
                      value={d.value}
                      onChange={(e) => update(i, { value: e.target.value })}
                      placeholder={d.op === 'IN' ? t('reportDetail.filters.inHint') : t('reportDetail.filters.valuePlaceholder')}
                      className={`${inputCls} flex-1 min-w-[100px]`}
                    />
                  )}
                  <button onClick={() => removeFilter(i)} className="w-6 h-6 rounded-md flex items-center justify-center text-gray-600 hover:text-red-400 hover:bg-red-500/10 transition-premium">
                    <Trash size={14} />
                  </button>
                </div>

                {/* Targets: which dataset column(s) this control applies to */}
                <div className="pl-3 border-l-2 border-amber-500/30 space-y-2">
                  <span className="text-[9px] text-amber-500/80 uppercase tracking-wider font-medium">{t('reportDetail.globalFilters.appliesTo')}</span>
                  {d.targets.map((tg, ti) => (
                    <div key={ti} className="flex items-center gap-2">
                      <select
                        value={tg.datasource_id}
                        onChange={(e) => updateTarget(i, ti, { datasource_id: Number(e.target.value), column: '' })}
                        className={`${inputCls} max-w-[160px]`}
                      >
                        {datasources.map((ds) => (
                          <option key={ds.id} value={ds.datasource_id}>{ds.name}</option>
                        ))}
                      </select>
                      <input
                        list={`gcols-${i}-${ti}`}
                        value={tg.column}
                        onChange={(e) => updateTarget(i, ti, { column: e.target.value })}
                        placeholder={t('reportDetail.filters.column')}
                        className={`${inputCls} flex-1 min-w-[100px]`}
                      />
                      <datalist id={`gcols-${i}-${ti}`}>
                        {targetCols(tg.datasource_id).map((c) => <option key={c} value={c} />)}
                      </datalist>
                      <button onClick={() => removeTarget(i, ti)} className="w-5 h-5 rounded flex items-center justify-center text-gray-600 hover:text-red-400 hover:bg-red-500/10 transition-premium">
                        <X size={12} />
                      </button>
                    </div>
                  ))}
                  <button onClick={() => addTarget(i)} className="flex items-center gap-1.5 text-[10px] text-gray-500 hover:text-gray-200 transition-premium mt-1">
                    <Plus size={11} /> {t('reportDetail.globalFilters.addTarget')}
                  </button>
                </div>
              </div>
            );
          })}

          {error && <p className="text-[11px] text-red-400">{error}</p>}
        </div>

        <div className="flex items-center gap-3 px-5 py-4 border-t border-obsidian-700 bg-obsidian-800/30">
          <button onClick={addFilter} className="flex items-center gap-1.5 text-[11px] text-gray-400 hover:text-amber-400 transition-premium">
            <Plus size={13} weight="bold" /> {t('reportDetail.globalFilters.add')}
          </button>
          <div className="flex-1" />
          <button onClick={onClose} className="text-[11px] text-gray-500 hover:text-gray-300 px-3 py-1.5 rounded-md hover:bg-obsidian-700 transition-premium">
            {t('common.close')}
          </button>
          <button
            onClick={apply}
            disabled={saving}
            className="flex items-center gap-1.5 text-[11px] text-amber-500 hover:text-amber-400 disabled:opacity-50 bg-amber-500/10 hover:bg-amber-500/15 border border-amber-500/30 rounded-md px-3.5 py-1.5 transition-premium font-medium"
          >
            <Check size={13} weight="bold" /> {saving ? t('reportDetail.filters.saving') : t('reportDetail.filters.apply')}
          </button>
        </div>
      </div>
    </div>
  );
}
