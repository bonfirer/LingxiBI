import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Brain, Plus, Trash, PencilSimple, Check, X, CaretDown, CaretRight,
} from '@phosphor-icons/react';
import { knowledgeBaseApi, type KnowledgeEntry } from '../lib/api';
import { isAdmin } from '../lib/currentUser';

const CATEGORY_LABELS: Record<string, { label: string; color: string }> = {
  relation: { label: '表关系', color: 'bg-blue-400/10 text-blue-400 border-blue-400/20' },
  field: { label: '字段含义', color: 'bg-amber-500/10 text-amber-500 border-amber-500/20' },
  pattern: { label: '查询模式', color: 'bg-data-green/10 text-data-green border-data-green/20' },
  business: { label: '业务规则', color: 'bg-purple-400/10 text-purple-400 border-purple-400/20' },
};

export default function KnowledgeBasePanel({ datasourceId }: { datasourceId: number }) {
  const { t } = useTranslation();
  const admin = isAdmin();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [expanded, setExpanded] = useState(true);
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editTitle, setEditTitle] = useState('');
  const [editContent, setEditContent] = useState('');
  const [showAdd, setShowAdd] = useState(false);
  const [newTitle, setNewTitle] = useState('');
  const [newContent, setNewContent] = useState('');
  const [newCategory, setNewCategory] = useState('relation');

  const fetchEntries = useCallback(async () => {
    try {
      setLoading(true);
      const data = await knowledgeBaseApi.listByDatasource(datasourceId);
      setEntries(data);
    } catch { /* silent */ }
    finally { setLoading(false); }
  }, [datasourceId]);

  useEffect(() => { fetchEntries(); }, [fetchEntries]);

  const handleDelete = async (id: number) => {
    await knowledgeBaseApi.delete(id).catch(() => {});
    setEntries((prev) => prev.filter((e) => e.id !== id));
  };

  const handleSaveEdit = async () => {
    if (!editingId || !editTitle.trim()) { setEditingId(null); return; }
    await knowledgeBaseApi.update(editingId, { title: editTitle.trim(), content: editContent.trim() }).catch(() => {});
    setEditingId(null);
    await fetchEntries();
  };

  const handleAdd = async () => {
    if (!newTitle.trim() || !newContent.trim()) return;
    await knowledgeBaseApi.create({
      datasource_id: datasourceId,
      category: newCategory,
      title: newTitle.trim(),
      content: newContent.trim(),
      source: 'manual',
    }).catch(() => {});
    setNewTitle('');
    setNewContent('');
    setShowAdd(false);
    await fetchEntries();
  };

  // Group by category
  const grouped = new Map<string, KnowledgeEntry[]>();
  for (const e of entries) {
    if (!grouped.has(e.category)) grouped.set(e.category, []);
    grouped.get(e.category)!.push(e);
  }

  return (
    <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden">
      {/* Header */}
      <div
        className="flex items-center justify-between px-4 py-2.5 border-b border-obsidian-700 cursor-pointer hover:bg-obsidian-800/50 transition-premium"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-2">
          {expanded ? <CaretDown size={12} className="text-gray-500" /> : <CaretRight size={12} className="text-gray-500" />}
          <Brain size={14} className="text-amber-500" />
          <span className="text-xs font-semibold text-gray-200">{t('kb.title')}</span>
          <span className="text-[9px] text-gray-500">({entries.length})</span>
        </div>
        {admin && (
          <button
            onClick={(e) => { e.stopPropagation(); setShowAdd(true); setExpanded(true); }}
            className="text-amber-500 hover:text-amber-400 transition-premium"
          >
            <Plus size={14} />
          </button>
        )}
      </div>

      {expanded && (
        <div className="p-3 max-h-[400px] overflow-y-auto scrollbar-thin space-y-2">
          {loading && (
            <div className="flex items-center gap-2 py-4 justify-center">
              <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
              <span className="text-[10px] text-gray-500">{t('common.loading')}</span>
            </div>
          )}

          {/* Add form */}
          {showAdd && (
            <div className="bg-obsidian-800 border border-amber-500/30 rounded-lg p-3 space-y-2">
              <div className="flex items-center gap-2">
                <select
                  value={newCategory}
                  onChange={(e) => setNewCategory(e.target.value)}
                  className="bg-obsidian-900 border border-obsidian-700 rounded px-2 py-1 text-[10px] text-gray-300 focus:outline-none"
                >
                  {Object.entries(CATEGORY_LABELS).map(([key, { label }]) => (
                    <option key={key} value={key}>{label}</option>
                  ))}
                </select>
                <input
                  value={newTitle}
                  onChange={(e) => setNewTitle(e.target.value)}
                  placeholder={t('kb.titlePlaceholder')}
                  className="flex-1 bg-obsidian-900 border border-obsidian-700 rounded px-2 py-1 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50"
                />
              </div>
              <textarea
                value={newContent}
                onChange={(e) => setNewContent(e.target.value)}
                placeholder={t('kb.contentPlaceholder')}
                rows={2}
                className="w-full bg-obsidian-900 border border-obsidian-700 rounded px-2 py-1.5 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 resize-y"
              />
              <div className="flex items-center gap-2">
                <button onClick={handleAdd} className="text-[10px] text-amber-500 hover:text-amber-400 font-medium">{t('common.save')}</button>
                <button onClick={() => setShowAdd(false)} className="text-[10px] text-gray-500 hover:text-gray-300">{t('common.cancel')}</button>
              </div>
            </div>
          )}

          {/* Entries by category */}
          {!loading && entries.length === 0 && !showAdd && (
            <div className="text-center py-6">
              <Brain size={24} className="text-gray-700 mx-auto mb-2" />
              <p className="text-[10px] text-gray-500">{t('kb.empty')}</p>
              <p className="text-[9px] text-gray-600 mt-1">{t('kb.emptyHint')}</p>
            </div>
          )}

          {Array.from(grouped.entries()).map(([category, items]) => {
            const catInfo = CATEGORY_LABELS[category] || CATEGORY_LABELS.relation;
            return (
              <div key={category}>
                <div className="flex items-center gap-1.5 mb-1">
                  <span className={`text-[8px] px-1.5 py-0.5 rounded border ${catInfo.color}`}>
                    {catInfo.label}
                  </span>
                  <span className="text-[8px] text-gray-600">{items.length}</span>
                </div>
                <div className="space-y-1 ml-1">
                  {items.map((entry) => (
                    <div key={entry.id} className="group bg-obsidian-800/50 rounded-lg px-3 py-2 border border-obsidian-700/50 hover:border-obsidian-600 transition-premium">
                      {editingId === entry.id ? (
                        <div className="space-y-1.5">
                          <input
                            value={editTitle}
                            onChange={(e) => setEditTitle(e.target.value)}
                            className="w-full bg-obsidian-900 border border-amber-500/50 rounded px-2 py-1 text-[10px] text-gray-200 focus:outline-none"
                          />
                          <textarea
                            value={editContent}
                            onChange={(e) => setEditContent(e.target.value)}
                            rows={2}
                            className="w-full bg-obsidian-900 border border-obsidian-700 rounded px-2 py-1 text-[10px] text-gray-300 focus:outline-none resize-y"
                          />
                          <div className="flex gap-2">
                            <button onClick={handleSaveEdit} className="text-amber-500 hover:text-amber-400"><Check size={12} /></button>
                            <button onClick={() => setEditingId(null)} className="w-5 h-5 rounded flex items-center justify-center text-gray-500 hover:text-gray-300 hover:bg-obsidian-700 transition-premium"><X size={12} /></button>
                          </div>
                        </div>
                      ) : (
                        <>
                          <div className="flex items-start justify-between">
                            <span className="text-[10px] text-gray-200 font-medium">{entry.title}</span>
                            {admin && (
                              <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-premium">
                                <button
                                  onClick={() => { setEditingId(entry.id); setEditTitle(entry.title); setEditContent(entry.content); }}
                                  className="text-gray-600 hover:text-gray-300"
                                >
                                  <PencilSimple size={10} />
                                </button>
                                <button onClick={() => handleDelete(entry.id)} className="text-gray-600 hover:text-red-400">
                                  <Trash size={10} />
                                </button>
                              </div>
                            )}
                          </div>
                          <p className="text-[9px] text-gray-500 mt-0.5 leading-relaxed">{entry.content}</p>
                          <div className="flex items-center gap-2 mt-1">
                            <span className={`text-[8px] ${entry.confidence === 'high' ? 'text-data-green' : entry.confidence === 'medium' ? 'text-amber-500' : 'text-gray-500'}`}>
                              {entry.confidence}
                            </span>
                            <span className="text-[8px] text-gray-600">{entry.source === 'ai' ? '🤖 AI' : '✏️ 手动'}</span>
                          </div>
                        </>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
