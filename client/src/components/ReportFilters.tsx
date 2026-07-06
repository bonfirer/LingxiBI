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
        className="bg-obsidian-900 border border-obsidian-700 rounded-2xl w-[520px] max-h-[75vh] overflow-hidden shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3 border-b border-obsidian-700">
          <h2 className="text-sm font-semibold text-gray-200 flex items-center gap-1.5">
            <Funnel size={14} className="text-amber-500" /> {t('reportDetail.globalFilters.title')}
          </h2>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300"><X size={16} /></button>
        </div>

        <div className="p-4 overflow-y-auto max-h-[55vh] scrollbar-thin space-y-3">
          <p className="text-[10px] text-gray-500 leading-relaxed bg-obsidian-800/40 border border-obsidian-700/50 rounded-lg px-2.5 py-2">
            {t('reportDetail.globalFilters.hint')}
          </p>
          {drafts.length === 0 && (
            <p className="text-[11px] text-gray-600">{t('reportDetail.globalFilters.empty')}</p>
          )}

          {drafts.map((d, i) => {
            const targetCols = (dsId: number) => columnsOf(datasources.find((x) => x.datasource_id === dsId));
            return (
              <div key={d.key} className="p-2.5 rounded-lg bg-obsidian-800 border border-obsidian-700 space-y-2">
                <div className="flex items-center gap-1.5">
                  <input
                    value={d.label}
                    onChange={(e) => update(i, { label: e.target.value })}
                    placeholder={t('reportDetail.globalFilters.label')}
                    className={`${inputCls} w-28`}
                  />
                  <select value={d.op} onChange={(e) => update(i, { op: e.target.value as Op })} className={inputCls}>
                    {OPS.map((op) => <option key={op} value={op}>{op}</option>)}
                  </select>
                  {d.op === 'BETWEEN' ? (
                    <>
                      <input value={d.from} onChange={(e) => update(i, { from: e.target.value })} placeholder={t('reportDetail.filters.betweenFrom')} className={`${inputCls} w-20`} />
                      <input value={d.to} onChange={(e) => update(i, { to: e.target.value })} placeholder={t('reportDetail.filters.betweenTo')} className={`${inputCls} w-20`} />
                    </>
                  ) : (
                    <input
                      value={d.value}
                      onChange={(e) => update(i, { value: e.target.value })}
                      placeholder={d.op === 'IN' ? t('reportDetail.filters.inHint') : t('reportDetail.filters.valuePlaceholder')}
                      className={`${inputCls} flex-1 min-w-[80px]`}
                    />
                  )}
                  <button onClick={() => removeFilter(i)} className="text-gray-700 hover:text-red-400 transition-premium">
                    <Trash size={13} />
                  </button>
                </div>

                {/* Targets: which dataset column(s) this control applies to */}
                <div className="pl-2 border-l border-obsidian-700 space-y-1">
                  <span className="text-[9px] text-gray-500 uppercase tracking-wide">{t('reportDetail.globalFilters.appliesTo')}</span>
                  {d.targets.map((tg, ti) => (
                    <div key={ti} className="flex items-center gap-1.5">
                      <select
                        value={tg.datasource_id}
                        onChange={(e) => updateTarget(i, ti, { datasource_id: Number(e.target.value), column: '' })}
                        className={`${inputCls} max-w-[150px]`}
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
                        className={`${inputCls} flex-1 min-w-[80px]`}
                      />
                      <datalist id={`gcols-${i}-${ti}`}>
                        {targetCols(tg.datasource_id).map((c) => <option key={c} value={c} />)}
                      </datalist>
                      <button onClick={() => removeTarget(i, ti)} className="text-gray-700 hover:text-red-400 transition-premium">
                        <X size={11} />
                      </button>
                    </div>
                  ))}
                  <button onClick={() => addTarget(i)} className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 transition-premium">
                    <Plus size={10} /> {t('reportDetail.globalFilters.addTarget')}
                  </button>
                </div>
              </div>
            );
          })}

          {error && <p className="text-[11px] text-red-400">{error}</p>}
        </div>

        <div className="flex items-center gap-2 px-4 py-3 border-t border-obsidian-700">
          <button onClick={addFilter} className="flex items-center gap-1 text-[11px] text-gray-400 hover:text-gray-200 transition-premium">
            <Plus size={12} /> {t('reportDetail.globalFilters.add')}
          </button>
          <div className="flex-1" />
          <button onClick={onClose} className="text-[11px] text-gray-500 hover:text-gray-300 px-2 transition-premium">
            {t('common.close')}
          </button>
          <button
            onClick={apply}
            disabled={saving}
            className="flex items-center gap-1 text-[11px] text-amber-500/90 hover:text-amber-400 disabled:opacity-50 border border-amber-500/30 rounded-md px-2.5 py-1 transition-premium"
          >
            <Check size={12} /> {saving ? t('reportDetail.filters.saving') : t('reportDetail.filters.apply')}
          </button>
        </div>
      </div>
    </div>
  );
}
