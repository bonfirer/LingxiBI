import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Funnel, Plus, Trash, Check, Sparkle } from '@phosphor-icons/react';
import { reportsApi, type ReportDataSource, type FilterCondition } from '../lib/api';

type Op = FilterCondition['op'];

const OPS: Op[] = ['=', '!=', '>', '>=', '<', '<=', 'LIKE', 'IN', 'BETWEEN'];

/** A locally-editable filter row. Values are kept as raw text and coerced to
 *  JSON (number vs string) only when applied. */
interface Draft {
  column: string;
  op: Op;
  value: string;   // scalar / LIKE / comma-separated IN
  from: string;    // BETWEEN lower bound
  to: string;      // BETWEEN upper bound
}

/** Coerce a text token to a number when it looks numeric, otherwise keep it a
 *  string. Keeps numeric comparisons numeric while leaving dates/text intact. */
function coerce(raw: string): string | number {
  const s = raw.trim();
  if (s !== '' && !Number.isNaN(Number(s))) return Number(s);
  return s;
}

function draftToCondition(d: Draft): FilterCondition | null {
  const column = d.column.trim();
  if (!column) return null;
  if (d.op === 'IN') {
    const values = d.value.split(',').map((v) => coerce(v)).filter((v) => v !== '');
    if (values.length === 0) return null;
    return { column, op: 'IN', value: values };
  }
  if (d.op === 'BETWEEN') {
    if (d.from.trim() === '' || d.to.trim() === '') return null;
    return { column, op: 'BETWEEN', value: [coerce(d.from), coerce(d.to)] };
  }
  if (d.value.trim() === '') return null;
  return { column, op: d.op, value: coerce(d.value) };
}

function conditionToDraft(c: FilterCondition): Draft {
  const base: Draft = { column: c.column, op: c.op, value: '', from: '', to: '' };
  if (c.op === 'IN' && Array.isArray(c.value)) {
    base.value = (c.value as unknown[]).join(', ');
  } else if (c.op === 'BETWEEN' && Array.isArray(c.value)) {
    base.from = String((c.value as unknown[])[0] ?? '');
    base.to = String((c.value as unknown[])[1] ?? '');
  } else {
    base.value = c.value == null ? '' : String(c.value);
  }
  return base;
}

interface Props {
  reportId: number;
  ds: ReportDataSource;
  onUpdated: (ds: ReportDataSource) => void;
}

/** Compact per-dataset filter editor. Filters are applied by the server by
 *  wrapping the metric SQL as a subquery; an empty list reverts to plain SQL. */
export function DatasourceFilters({ reportId, ds, onUpdated }: Props) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Draft[]>(() =>
    (ds.filters ?? []).map(conditionToDraft),
  );
  // Natural-language instruction for AI-suggested conditions.
  const [aiInput, setAiInput] = useState('');
  const [aiLoading, setAiLoading] = useState(false);

  // Column suggestions derived from the cached result's first row.
  const columns = useMemo(() => {
    const rows = ds.result_cache;
    if (Array.isArray(rows) && rows.length > 0 && typeof rows[0] === 'object' && rows[0]) {
      return Object.keys(rows[0] as Record<string, unknown>);
    }
    return [];
  }, [ds.result_cache]);

  const appliedCount = (ds.filters ?? []).length;

  const update = (i: number, patch: Partial<Draft>) =>
    setDrafts((prev) => prev.map((d, idx) => (idx === i ? { ...d, ...patch } : d)));

  const addRow = () =>
    setDrafts((prev) => [...prev, { column: columns[0] ?? '', op: '=', value: '', from: '', to: '' }]);

  const removeRow = (i: number) => setDrafts((prev) => prev.filter((_, idx) => idx !== i));

  // Ask the AI to turn the instruction into conditions and append them as
  // editable draft rows (the user reviews, then clicks Apply).
  const askAi = async () => {
    const instruction = aiInput.trim();
    if (!instruction || aiLoading) return;
    setAiLoading(true);
    setError(null);
    try {
      const suggested = await reportsApi.aiDatasourceFilters(reportId, ds.id, instruction);
      if (suggested.length === 0) {
        setError(t('reportDetail.filters.aiNone'));
      } else {
        setDrafts((prev) => [...prev, ...suggested.map(conditionToDraft)]);
        setAiInput('');
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setAiLoading(false);
    }
  };

  const apply = async () => {
    setSaving(true);
    setError(null);
    try {
      const conditions = drafts.map(draftToCondition).filter((c): c is FilterCondition => c !== null);
      const updated = await reportsApi.setDatasourceFilters(reportId, ds.id, conditions);
      onUpdated(updated);
      setDrafts((updated.filters ?? []).map(conditionToDraft));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const clear = async () => {
    setSaving(true);
    setError(null);
    try {
      const updated = await reportsApi.setDatasourceFilters(reportId, ds.id, []);
      onUpdated(updated);
      setDrafts([]);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  const inputCls =
    'bg-obsidian-900 border border-obsidian-700 rounded px-1.5 py-1 text-[11px] text-gray-200 focus:outline-none focus:border-amber-500/50';

  return (
    <div className="mt-1">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex items-center gap-1 text-[9px] text-gray-500 hover:text-amber-500/90 transition-premium"
      >
        <Funnel size={10} weight={appliedCount > 0 ? 'fill' : 'regular'} className={appliedCount > 0 ? 'text-amber-500' : ''} />
        {t('reportDetail.filters.title')}
        {appliedCount > 0 && (
          <span className="text-amber-500/80">· {t('reportDetail.filters.applied', { count: appliedCount })}</span>
        )}
      </button>

      {open && (
        <div className="mt-1.5 p-2 rounded-lg bg-obsidian-900/60 border border-obsidian-700/60 space-y-1.5">
          {/* AI: describe conditions in natural language */}
          <div className="flex items-center gap-1">
            <Sparkle size={11} className="text-amber-500 flex-shrink-0" />
            <input
              value={aiInput}
              onChange={(e) => setAiInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') askAi(); }}
              placeholder={t('reportDetail.filters.aiPlaceholder')}
              className={`${inputCls} flex-1 min-w-[80px]`}
            />
            <button
              onClick={askAi}
              disabled={aiLoading || !aiInput.trim()}
              className="flex items-center gap-1 text-[10px] text-amber-500/90 hover:text-amber-400 disabled:opacity-40 transition-premium flex-shrink-0"
            >
              {aiLoading ? t('reportDetail.filters.aiThinking') : t('reportDetail.filters.aiGenerate')}
            </button>
          </div>
          <div className="border-t border-obsidian-700/50" />

          {drafts.length === 0 && (
            <p className="text-[10px] text-gray-600">{t('reportDetail.filters.empty')}</p>
          )}

          {drafts.map((d, i) => (
            <div key={i} className="flex flex-wrap items-center gap-1">
              {columns.length > 0 ? (
                <input
                  list={`cols-${ds.id}`}
                  value={d.column}
                  onChange={(e) => update(i, { column: e.target.value })}
                  placeholder={t('reportDetail.filters.column')}
                  className={`${inputCls} w-24`}
                />
              ) : (
                <input
                  value={d.column}
                  onChange={(e) => update(i, { column: e.target.value })}
                  placeholder={t('reportDetail.filters.column')}
                  className={`${inputCls} w-24`}
                />
              )}
              <select
                value={d.op}
                onChange={(e) => update(i, { op: e.target.value as Op })}
                className={inputCls}
              >
                {OPS.map((op) => (
                  <option key={op} value={op}>{op}</option>
                ))}
              </select>

              {d.op === 'BETWEEN' ? (
                <>
                  <input
                    value={d.from}
                    onChange={(e) => update(i, { from: e.target.value })}
                    placeholder={t('reportDetail.filters.betweenFrom')}
                    className={`${inputCls} w-20`}
                  />
                  <input
                    value={d.to}
                    onChange={(e) => update(i, { to: e.target.value })}
                    placeholder={t('reportDetail.filters.betweenTo')}
                    className={`${inputCls} w-20`}
                  />
                </>
              ) : (
                <input
                  value={d.value}
                  onChange={(e) => update(i, { value: e.target.value })}
                  placeholder={d.op === 'IN' ? t('reportDetail.filters.inHint') : t('reportDetail.filters.valuePlaceholder')}
                  className={`${inputCls} flex-1 min-w-[80px]`}
                />
              )}

              <button onClick={() => removeRow(i)} className="text-gray-700 hover:text-red-400 transition-premium">
                <Trash size={12} />
              </button>
            </div>
          ))}

          {columns.length > 0 && (
            <datalist id={`cols-${ds.id}`}>
              {columns.map((c) => <option key={c} value={c} />)}
            </datalist>
          )}

          {error && <p className="text-[10px] text-red-400">{error}</p>}

          <div className="flex items-center gap-2 pt-0.5">
            <button
              onClick={addRow}
              className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 transition-premium"
            >
              <Plus size={11} /> {t('reportDetail.filters.add')}
            </button>
            <div className="flex-1" />
            {appliedCount > 0 && (
              <button
                onClick={clear}
                disabled={saving}
                className="text-[10px] text-gray-500 hover:text-gray-300 disabled:opacity-50 transition-premium"
              >
                {t('reportDetail.filters.clear')}
              </button>
            )}
            <button
              onClick={apply}
              disabled={saving}
              className="flex items-center gap-1 text-[10px] text-amber-500/90 hover:text-amber-400 disabled:opacity-50 transition-premium"
            >
              <Check size={11} /> {saving ? t('reportDetail.filters.saving') : t('reportDetail.filters.apply')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
