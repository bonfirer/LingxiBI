import { useEffect, useState, useCallback } from 'react';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Plus,
  CaretRight,
  CaretDown,
  Folder,
  Star,
  MagnifyingGlass,
  Database,
  ChartBar,
  Trash,
  X,
  ListBullets,
} from '@phosphor-icons/react';
import {
  datasourcesApi,
  reportsApi,
  reportGroupsApi,
  metricsApi,
  metricGroupsApi,
  type SchemaInfo,
  type Report,
  type ReportGroup,
  type MetricPool,
  type MetricGroup,
} from '../lib/api';
import { useDataSourceStore } from '../stores/datasourceStore';
import { isAdmin } from '../lib/currentUser';

const statusColors: Record<string, string> = {
  connected: 'bg-data-green',
  warning: 'bg-data-amber',
  error: 'bg-red-500',
  unknown: 'bg-gray-600',
};

// Status dot for a report's generation state
function ReportStatusDot({ report }: { report: Report }) {
  const status = report.generation_status;
  if (status === 'generating') {
    return <span className="w-1.5 h-1.5 rounded-full bg-amber-500 dot-breathe flex-shrink-0" title="生成中" />;
  }
  if (status === 'failed') {
    return <span className="w-1.5 h-1.5 rounded-full bg-red-500 flex-shrink-0" title="生成失败" />;
  }
  if (report.html_content || status === 'done') {
    return <span className="w-1.5 h-1.5 rounded-full bg-data-green flex-shrink-0" title="已生成" />;
  }
  return <span className="w-1.5 h-1.5 rounded-full bg-gray-600 flex-shrink-0" title="未生成" />;
}

export default function AssetPanel() {
  const { t } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  // Mobile: the asset panel is a slide-in drawer toggled by a floating button.
  const [open, setOpen] = useState(false);

  const path = location.pathname;
  const section = path.startsWith('/datasources')
    ? 'datasources'
    : path.startsWith('/reports')
      ? 'reports'
      : path.startsWith('/metrics')
        ? 'metrics'
        : path.startsWith('/conversations')
          ? 'conversations'
          : 'default';

  // Close the drawer on navigation so a selection reveals the content.
  const nav = (p: string) => { navigate(p); setOpen(false); };

  const body = (
    <div className="px-2.5 pb-2.5 flex-1 overflow-y-auto scrollbar-thin">
      {section === 'datasources' && <DataSourcesKGPanel t={t} navigate={nav} />}
      {section === 'reports' && <ReportsPanel t={t} navigate={nav} />}
      {section === 'metrics' && <MetricsPanel t={t} navigate={nav} />}
      {section === 'conversations' && <ConversationsPanel t={t} />}
      {section === 'default' && <ReportsPanel t={t} navigate={nav} />}
    </div>
  );

  return (
    <>
      {/* Desktop: static side panel */}
      <aside className="hidden md:flex w-[190px] bg-obsidian-900 border-r border-obsidian-700 flex-col flex-shrink-0 overflow-hidden">
        {body}
      </aside>

      {/* Mobile: floating toggle button (sits above the bottom nav) */}
      <button
        onClick={() => setOpen(true)}
        className="md:hidden fixed bottom-[68px] right-4 z-40 w-11 h-11 rounded-full bg-amber-500 text-[#08080c] shadow-lg flex items-center justify-center active:scale-95 transition-transform"
        aria-label={t('assetPanel.browse')}
        title={t('assetPanel.browse')}
      >
        <ListBullets size={20} weight="bold" />
      </button>

      {/* Mobile: slide-in drawer */}
      {open && (
        <div
          className="md:hidden fixed inset-0 z-[60] bg-black/60"
          onClick={() => setOpen(false)}
        />
      )}
      <aside
        className={`
          md:hidden fixed top-0 bottom-14 left-0 z-[70] w-64 bg-obsidian-900 border-r border-obsidian-700
          flex flex-col overflow-hidden transform transition-transform duration-200
          ${open ? 'translate-x-0' : '-translate-x-full'}
        `}
      >
        <div className="flex items-center justify-end px-2 py-2 border-b border-obsidian-700 flex-shrink-0">
          <button
            onClick={() => setOpen(false)}
            className="w-7 h-7 flex items-center justify-center text-gray-500 hover:text-gray-300 rounded-md hover:bg-obsidian-800"
            aria-label={t('common.close')}
          >
            <X size={16} />
          </button>
        </div>
        {body}
      </aside>
    </>
  );
}

// ── Data Sources + Knowledge Graph Panel (combined) ──
function DataSourcesKGPanel({ t, navigate }: { t: (k: string) => string; navigate: (p: string) => void }) {
  const { sources, setSources } = useDataSourceStore();
  const [searchParams] = useSearchParams();
  const activeDsId = searchParams.get('ds') ? parseInt(searchParams.get('ds')!) : null;
  const [schemas, setSchemas] = useState<Record<number, SchemaInfo>>({});
  const [loadingSchema, setLoadingSchema] = useState<number | null>(null);
  const [expandedTables, setExpandedTables] = useState<Set<string>>(new Set());

  useEffect(() => { datasourcesApi.list().then(setSources).catch(() => {}); }, [setSources]);

  // Load schema when a datasource is selected
  useEffect(() => {
    if (activeDsId && !schemas[activeDsId]) {
      setLoadingSchema(activeDsId);
      datasourcesApi.getSchema(activeDsId)
        .then((s) => setSchemas((prev) => ({ ...prev, [activeDsId]: s })))
        .catch(() => {})
        .finally(() => setLoadingSchema(null));
    }
  }, [activeDsId]);

  const toggleTable = (key: string) => {
    setExpandedTables((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const activeSchema = activeDsId ? schemas[activeDsId] : null;

  return (
    <>
      {/* Header with add button */}
      <div className="sticky top-0 z-10 bg-obsidian-900 -mx-2.5 px-2.5 pt-2.5 pb-1.5 flex items-center justify-between">
        <span className="text-amber-500 text-[10px] font-bold tracking-wide uppercase">
          {t('assetPanel.dataSources')}
        </span>
        {isAdmin() && (
          <button
            onClick={() => { navigate('/datasources?new=1'); }}
            className="text-amber-500 hover:text-amber-400 transition-premium"
            title={t('datasources.newSource')}
          >
            <Plus size={14} weight="bold" />
          </button>
        )}
      </div>

      {sources.length === 0 && (
        <p className="text-[9px] text-gray-600 italic px-1">{t('assetPanel.noSources')}</p>
      )}

      {/* Datasource list */}
      {sources.map((ds) => {
        const isActive = activeDsId === ds.id;
        return (
          <div key={ds.id} className="mb-0.5">
            <div
              onClick={() => navigate(`/datasources?ds=${ds.id}`)}
              className={`flex items-center gap-1.5 px-2 py-1.5 rounded-md transition-premium cursor-pointer border-l-2 ${
                isActive
                  ? 'bg-amber-500/10 border-amber-500'
                  : 'border-transparent hover:bg-obsidian-800'
              }`}
            >
              <Database size={12} className={isActive ? 'text-amber-500 flex-shrink-0' : 'text-gray-500 flex-shrink-0'} />
              <div className="min-w-0 flex-1">
                <div className={`text-[11px] font-medium truncate ${isActive ? 'text-amber-500' : 'text-gray-300'}`}>{ds.name}</div>
                <div className="text-[9px] text-gray-500">{ds.db_type}</div>
              </div>
              <div className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${statusColors[ds.status] || statusColors.unknown}`} />
            </div>
          </div>
        );
      })}

      {/* Schema tree for active datasource */}
      {activeDsId && (
        <div className="mt-2 pt-2 border-t border-obsidian-700">
          <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium block mb-1 px-1">
            {t('kg.tables')}
            {activeSchema && (
              <span className="text-gray-600 ml-1">({activeSchema.tables.length})</span>
            )}
          </span>

          {loadingSchema === activeDsId && (
            <div className="flex items-center gap-1.5 px-1 py-2">
              <div className="w-2.5 h-2.5 border border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
              <span className="text-[9px] text-gray-600">{t('common.loading')}</span>
            </div>
          )}

          {!activeSchema && loadingSchema !== activeDsId && (
            <p className="text-[9px] text-gray-600 italic px-1">{t('kg.noSchema')}</p>
          )}

          {activeSchema && activeSchema.tables.map((table) => {
            const tableKey = `${activeDsId}-${table.name}`;
            const isExpanded = expandedTables.has(tableKey);

            return (
              <div key={tableKey} className="mb-0.5">
                <div
                  onClick={() => toggleTable(tableKey)}
                  className="flex items-center gap-1 px-1 py-1 rounded hover:bg-obsidian-800 cursor-pointer transition-premium"
                >
                  {isExpanded
                    ? <CaretDown size={9} className="text-gray-600 flex-shrink-0" />
                    : <CaretRight size={9} className="text-gray-600 flex-shrink-0" />
                  }
                  <span className="text-[10px] text-gray-300 font-mono truncate flex-1" title={table.comment || undefined}>{table.name}</span>
                  <span className="text-[8px] text-gray-600 flex-shrink-0">{table.columns.length}</span>
                </div>

                {isExpanded && (
                  <div className="ml-3 border-l border-obsidian-700 pl-1.5">
                    {table.columns.map((col) => (
                      <div
                        key={col.name}
                        className="flex items-center gap-1 px-1 py-0.5 text-[9px]"
                        title={`${col.name} ${col.data_type}${col.nullable ? ' NULL' : ' NOT NULL'}${col.is_primary_key ? ' PK' : ''}${col.is_foreign_key ? ' FK' : ''}${col.comment ? '\n' + col.comment : ''}`}
                      >
                        {col.is_primary_key ? (
                          <span className="text-amber-500 font-bold flex-shrink-0 w-3 text-center">K</span>
                        ) : col.is_foreign_key ? (
                          <span className="text-blue-400 font-bold flex-shrink-0 w-3 text-center">F</span>
                        ) : (
                          <span className="text-gray-700 flex-shrink-0 w-3 text-center">·</span>
                        )}
                        <span className="text-gray-400 font-mono truncate flex-1">{col.name}</span>
                        <span className="text-gray-600 font-mono flex-shrink-0 text-[8px]">
                          {col.data_type.length > 12 ? col.data_type.slice(0, 12) : col.data_type}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Saved SQL Snippets */}
      <SqlFavorites activeDsId={activeDsId} />
    </>
  );
}

// ── SQL Favorites (saved snippets in localStorage) ──

interface SqlSnippet {
  id: string;
  name: string;
  sql: string;
  dsId: number | null;
  createdAt: number;
}

function getSavedSnippets(): SqlSnippet[] {
  try {
    const raw = localStorage.getItem('sql-favorites');
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
}

function saveSnippets(snippets: SqlSnippet[]) {
  localStorage.setItem('sql-favorites', JSON.stringify(snippets));
}

function SqlFavorites({ activeDsId }: { activeDsId: number | null }) {
  const [snippets, setSnippets] = useState<SqlSnippet[]>(getSavedSnippets);
  const [expanded, setExpanded] = useState(false);

  // Listen for save events from SQL panel
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { name: string; sql: string; dsId: number | null };
      const newSnippet: SqlSnippet = {
        id: `${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
        name: detail.name,
        sql: detail.sql,
        dsId: detail.dsId,
        createdAt: Date.now(),
      };
      const updated = [newSnippet, ...getSavedSnippets()];
      saveSnippets(updated);
      setSnippets(updated);
      setExpanded(true);
    };
    window.addEventListener('sql-favorite-save', handler);
    return () => window.removeEventListener('sql-favorite-save', handler);
  }, []);

  const handleDelete = (id: string) => {
    const updated = snippets.filter((s) => s.id !== id);
    saveSnippets(updated);
    setSnippets(updated);
  };

  const handleClick = (sql: string) => {
    // Dispatch event to fill SQL panel
    window.dispatchEvent(new CustomEvent('sql-favorite-load', { detail: { sql } }));
  };

  const filtered = activeDsId
    ? snippets.filter((s) => s.dsId === activeDsId || s.dsId === null)
    : snippets;

  if (filtered.length === 0 && !expanded) return null;

  return (
    <div className="mt-2 pt-2 border-t border-obsidian-700">
      <div
        className="flex items-center gap-1 px-1 mb-1 cursor-pointer"
        onClick={() => setExpanded(!expanded)}
      >
        {expanded
          ? <CaretDown size={9} className="text-gray-600" />
          : <CaretRight size={9} className="text-gray-600" />
        }
        <Star size={10} className="text-amber-500/60" weight="fill" />
        <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium flex-1">
          收藏
        </span>
        <span className="text-[8px] text-gray-600">{filtered.length}</span>
      </div>

      {expanded && filtered.length === 0 && (
        <p className="text-[9px] text-gray-600 italic px-1">暂无收藏的 SQL</p>
      )}

      {expanded && filtered.map((s) => (
        <div
          key={s.id}
          className="flex items-center gap-1 px-1 py-1 rounded hover:bg-obsidian-800 cursor-pointer transition-premium group"
          onClick={() => handleClick(s.sql)}
          title={s.sql}
        >
          <span className="text-[10px] text-gray-400 font-mono truncate flex-1">{s.name}</span>
          <button
            onClick={(e) => { e.stopPropagation(); handleDelete(s.id); }}
            className="text-gray-700 hover:text-red-400 opacity-0 group-hover:opacity-100 transition-premium flex-shrink-0"
          >
            <Trash size={10} />
          </button>
        </div>
      ))}
    </div>
  );
}

// ── Reports Panel (grouped tree) ──
function ReportsPanel({ t, navigate }: { t: (k: string) => string; navigate: (p: string) => void }) {
  const [reports, setReports] = useState<Report[]>([]);
  const [groups, setGroups] = useState<ReportGroup[]>([]);
  const [collapsed, setCollapsed] = useState<Set<number | null>>(new Set());
  const [editingGroupId, setEditingGroupId] = useState<number | null>(null);
  const [editingGroupName, setEditingGroupName] = useState('');
  const [showNewGroup, setShowNewGroup] = useState(false);
  const [newGroupName, setNewGroupName] = useState('');
  const [showNewReport, setShowNewReport] = useState(false);
  const [newReportName, setNewReportName] = useState('');
  const [activeGroupId, setActiveGroupId] = useState<number | null>(null);
  const [dragOverGroup, setDragOverGroup] = useState<number | null | undefined>(undefined);
  const [search, setSearch] = useState('');

  const fetchData = useCallback(async () => {
    const [r, g] = await Promise.all([reportsApi.list(), reportGroupsApi.list()]);
    setReports(r);
    setGroups(g);
  }, []);

  useEffect(() => { fetchData().catch(() => {}); }, [fetchData]);

  // Listen for report changes from other components
  useEffect(() => {
    const handler = () => { fetchData().catch(() => {}); };
    window.addEventListener('reports-updated', handler);
    return () => window.removeEventListener('reports-updated', handler);
  }, [fetchData]);

  // Poll while any report is generating, to update status dots live
  useEffect(() => {
    const anyGenerating = reports.some((r) => r.generation_status === 'generating');
    if (!anyGenerating) return;
    const timer = setInterval(() => { fetchData().catch(() => {}); }, 4000);
    return () => clearInterval(timer);
  }, [reports, fetchData]);

  const toggle = (id: number | null) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  const handleDropReport = async (e: React.DragEvent, groupId: number | null) => {
    e.preventDefault();
    setDragOverGroup(undefined);
    const reportId = parseInt(e.dataTransfer.getData('reportId'));
    if (!isNaN(reportId)) {
      await reportsApi.move(reportId, groupId).catch(() => {});
      await fetchData().catch(() => {});
    }
  };

  const handleDeleteGroup = async (id: number) => {
    await reportGroupsApi.delete(id).catch(() => {});
    if (activeGroupId === id) setActiveGroupId(null);
    await fetchData().catch(() => {});
  };

  const handleRenameGroup = async (id: number) => {
    const trimmed = editingGroupName.trim();
    if (trimmed) {
      await reportGroupsApi.update(id, { name: trimmed }).catch(() => {});
      await fetchData().catch(() => {});
    }
    setEditingGroupId(null);
  };

  const handleCreateGroup = async () => {
    if (!newGroupName.trim()) return;
    await reportGroupsApi.create({ name: newGroupName.trim() }).catch(() => {});
    setNewGroupName('');
    setShowNewGroup(false);
    await fetchData().catch(() => {});
  };

  const handleCreateReport = async () => {
    if (!newReportName.trim()) return;
    const report = await reportsApi.create({
      title: newReportName.trim(),
      pool_ids: [],
      group_id: activeGroupId,
    }).catch(() => null);
    setNewReportName('');
    setShowNewReport(false);
    await fetchData().catch(() => {});
    if (report) navigate(`/reports/${report.id}`);
  };

  // Group reports
  const grouped = new Map<number | null, Report[]>();
  for (const g of groups) grouped.set(g.id, []);
  grouped.set(null, []);
  for (const r of reports) {
    const key = r.group_id ?? null;
    if (!grouped.has(key)) grouped.set(key, []);
    grouped.get(key)!.push(r);
  }

  return (
    <>
      <div className="sticky top-0 z-10 bg-obsidian-900 -mx-2.5 px-2.5 pt-2.5">
        <div className="flex items-center justify-between mb-1.5">
          <span className="text-amber-500 text-[10px] font-bold tracking-wide uppercase">{t('nav.reports')}</span>
          <div className="flex items-center gap-1">
            <button onClick={() => { setShowNewReport(true); if (activeGroupId !== null) { setCollapsed((prev) => { const next = new Set(prev); next.delete(activeGroupId); return next; }); } }} className="text-amber-500 hover:text-amber-400 transition-premium" title={t('reports.newReport')}>
              <Plus size={13} weight="bold" />
            </button>
            <button onClick={() => setShowNewGroup(true)} className="text-gray-500 hover:text-gray-300 transition-premium" title={t('reports.newGroup')}>
              <Folder size={12} />
            </button>
          </div>
        </div>

        {/* Search */}
        <div className="relative mb-2">
          <MagnifyingGlass size={11} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-600" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t('reports.searchPlaceholder') || '搜索报表...'}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-md pl-6 pr-2 py-1 text-[10px] text-gray-300 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
          />
          {search && (
            <button onClick={() => setSearch('')} className="absolute right-1.5 top-1/2 -translate-y-1/2 text-gray-600 hover:text-gray-400">
              <X size={10} />
            </button>
          )}
        </div>
      </div>

      {/* Search results */}
      {search.trim() ? (
        <div className="space-y-0.5 mb-2">
          {reports
            .filter((r) => r.title.toLowerCase().includes(search.toLowerCase()))
            .map((r) => (
              <div
                key={r.id}
                onClick={() => { navigate(`/reports/${r.id}`); setSearch(''); }}
                className="flex items-center gap-1.5 text-[10px] text-gray-400 hover:text-gray-200 px-2 py-1.5 rounded hover:bg-obsidian-800 transition-premium cursor-pointer"
              >
                <ReportStatusDot report={r} />
                <span className="truncate flex-1">{r.title}</span>
                <span className="text-[8px] text-gray-600 flex-shrink-0">
                  {groups.find((g) => g.id === r.group_id)?.name || ''}
                </span>
              </div>
            ))}
          {reports.filter((r) => r.title.toLowerCase().includes(search.toLowerCase())).length === 0 && (
            <p className="text-[9px] text-gray-600 italic px-1">无匹配结果</p>
          )}
        </div>
      ) : (
      <>

      {/* New group inline form */}
      {showNewGroup && (
        <div className="flex items-center gap-1 mb-1.5 px-1">
          <Folder size={10} className="text-amber-500/60 flex-shrink-0" />
          <input
            value={newGroupName}
            onChange={(e) => setNewGroupName(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleCreateGroup(); if (e.key === 'Escape') setShowNewGroup(false); }}
            placeholder={t('reports.groupNamePlaceholder')}
            autoFocus
            className="flex-1 bg-obsidian-800 border border-amber-500/50 rounded px-1.5 py-0.5 text-[9px] text-gray-200 placeholder-gray-600 focus:outline-none min-w-0"
          />
        </div>
      )}

      {/* New report form — shown at top only if no group is active */}
      {showNewReport && activeGroupId === null && (
        <div className="flex items-center gap-1 mb-1.5 px-1">
          <ChartBar size={10} className="text-amber-500/60 flex-shrink-0" />
          <input
            value={newReportName}
            onChange={(e) => setNewReportName(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleCreateReport(); if (e.key === 'Escape') setShowNewReport(false); }}
            placeholder={t('reports.reportNamePlaceholder')}
            autoFocus
            className="flex-1 bg-obsidian-800 border border-amber-500/50 rounded px-1.5 py-0.5 text-[9px] text-gray-200 placeholder-gray-600 focus:outline-none min-w-0"
          />
        </div>
      )}

      {groups.map((group) => {
        const items = grouped.get(group.id) || [];
        const isCollapsed = collapsed.has(group.id);
        const isEditing = editingGroupId === group.id;
        const isActive = activeGroupId === group.id;
        const isDragOver = dragOverGroup === group.id;
        return (
          <div
            key={group.id}
            className={`mb-1 rounded transition-all group/rg ${isDragOver ? 'ring-1 ring-amber-500/30 bg-amber-500/5' : ''}`}
            onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; setDragOverGroup(group.id); }}
            onDragLeave={() => setDragOverGroup(undefined)}
            onDrop={(e) => handleDropReport(e, group.id)}
          >
            <div
              className={`flex items-center gap-1 px-1 py-1 rounded cursor-pointer transition-premium ${
                isActive ? 'bg-amber-500/10 ring-1 ring-amber-500/20' : 'hover:bg-obsidian-800'
              }`}
              onClick={() => { toggle(group.id); setActiveGroupId(group.id); }}
            >
              {isCollapsed ? <CaretRight size={10} className="text-gray-600" /> : <CaretDown size={10} className="text-gray-600" />}
              <Folder size={11} className={isActive || isDragOver ? 'text-amber-500' : 'text-amber-500/60'} />
              {isEditing ? (
                <input
                  value={editingGroupName}
                  onChange={(e) => setEditingGroupName(e.target.value)}
                  onBlur={() => handleRenameGroup(group.id)}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleRenameGroup(group.id); if (e.key === 'Escape') setEditingGroupId(null); }}
                  onClick={(e) => e.stopPropagation()}
                  autoFocus
                  className="text-[10px] text-gray-200 bg-obsidian-800 border border-amber-500/50 rounded px-1 py-0 flex-1 min-w-0 focus:outline-none"
                />
              ) : (
                <span
                  className={`text-[10px] font-medium truncate flex-1 ${isActive ? 'text-amber-500' : 'text-gray-300'}`}
                  onDoubleClick={(e) => { e.stopPropagation(); setEditingGroupId(group.id); setEditingGroupName(group.name); }}
                  title="Double-click to rename"
                >
                  {group.name}
                </span>
              )}
              <span className="text-[8px] text-gray-600">{items.length}</span>
              <button
                onClick={(e) => { e.stopPropagation(); handleDeleteGroup(group.id); }}
                className="text-gray-700 hover:text-red-400 opacity-0 group-hover/rg:opacity-100 transition-premium flex-shrink-0"
                title={t('common.delete')}
              >
                <Trash size={10} />
              </button>
            </div>
            {/* New report input inside active group */}
            {!isCollapsed && showNewReport && isActive && (
              <div className="flex items-center gap-1 pl-5 pr-1 py-1">
                <ChartBar size={9} className="text-amber-500/60 flex-shrink-0" />
                <input
                  value={newReportName}
                  onChange={(e) => setNewReportName(e.target.value)}
                  onKeyDown={(e) => { if (e.key === 'Enter') handleCreateReport(); if (e.key === 'Escape') setShowNewReport(false); }}
                  placeholder={t('reports.reportNamePlaceholder')}
                  autoFocus
                  className="flex-1 bg-obsidian-800 border border-amber-500/50 rounded px-1.5 py-0.5 text-[9px] text-gray-200 placeholder-gray-600 focus:outline-none min-w-0"
                />
              </div>
            )}
            {!isCollapsed && items.map((r) => (
              <div
                key={r.id}
                draggable
                onDragStart={(e) => { e.dataTransfer.setData('reportId', String(r.id)); e.dataTransfer.effectAllowed = 'move'; }}
                onClick={() => navigate(`/reports/${r.id}`)}
                className="flex items-center gap-1.5 text-[10px] text-gray-400 hover:text-gray-200 pl-6 pr-1.5 py-1 rounded hover:bg-obsidian-800 transition-premium cursor-grab active:cursor-grabbing"
              >
                <ReportStatusDot report={r} />
                <span className="truncate flex-1">{r.title}</span>
              </div>
            ))}
          </div>
        );
      })}

      {/* Ungrouped */}
      {(grouped.get(null) || []).length > 0 && (
        <div
          className={`mb-1 rounded transition-all ${dragOverGroup === null ? 'ring-1 ring-amber-500/30 bg-amber-500/5' : ''}`}
          onDragOver={(e) => { e.preventDefault(); e.dataTransfer.dropEffect = 'move'; setDragOverGroup(null); }}
          onDragLeave={() => setDragOverGroup(undefined)}
          onDrop={(e) => handleDropReport(e, null)}
        >
          <div
            className="flex items-center gap-1 px-1 py-1 rounded hover:bg-obsidian-800 cursor-pointer transition-premium"
            onClick={() => { toggle(null); setActiveGroupId(null); }}
          >
            {collapsed.has(null) ? <CaretRight size={10} className="text-gray-600" /> : <CaretDown size={10} className="text-gray-600" />}
            <span className="text-[10px] text-gray-500 flex-1">{t('metrics.ungrouped')}</span>
            <span className="text-[8px] text-gray-600">{(grouped.get(null) || []).length}</span>
          </div>
          {!collapsed.has(null) && (grouped.get(null) || []).map((r) => (
            <div
              key={r.id}
              draggable
              onDragStart={(e) => { e.dataTransfer.setData('reportId', String(r.id)); e.dataTransfer.effectAllowed = 'move'; }}
              onClick={() => navigate(`/reports/${r.id}`)}
              className="flex items-center gap-1.5 text-[10px] text-gray-400 hover:text-gray-200 pl-5 pr-1.5 py-1 rounded hover:bg-obsidian-800 transition-premium cursor-grab active:cursor-grabbing"
            >
              <ReportStatusDot report={r} />
              <span className="truncate flex-1">{r.title}</span>
            </div>
          ))}
        </div>
      )}

      {reports.length === 0 && <p className="text-[9px] text-gray-600 italic px-1">{t('assetPanel.noReports')}</p>}
      </>
      )}
    </>
  );
}

// ── Metrics Panel (grouped tree with drag-drop & rename) ──
function MetricsPanel({ t, navigate }: { t: (k: string) => string; navigate: (p: string) => void }) {
  const [searchParams] = useSearchParams();
  const activeMetricId = searchParams.get('id') ? parseInt(searchParams.get('id')!) : null;
  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [groups, setGroups] = useState<MetricGroup[]>([]);
  const [collapsed, setCollapsed] = useState<Set<number | null>>(new Set());
  const [dragOverGroup, setDragOverGroup] = useState<number | null | undefined>(undefined);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editingValue, setEditingValue] = useState('');
  const [search, setSearch] = useState('');

  const fetchData = useCallback(async () => {
    const [m, g] = await Promise.all([metricsApi.list(), metricGroupsApi.list()]);
    setMetrics(m);
    setGroups(g);
  }, []);

  useEffect(() => { fetchData().catch(() => {}); }, [fetchData]);

  // Listen for refresh events from other components
  useEffect(() => {
    const handler = () => { fetchData().catch(() => {}); };
    window.addEventListener('metrics-updated', handler);
    return () => window.removeEventListener('metrics-updated', handler);
  }, [fetchData]);

  const toggle = (id: number | null) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      return next;
    });
  };

  // Rename handlers
  const startEdit = (key: string, currentValue: string) => {
    setEditingId(key);
    setEditingValue(currentValue);
  };

  const saveEdit = async () => {
    if (!editingId || !editingValue.trim()) { setEditingId(null); return; }
    const [type, idStr] = editingId.split('-');
    const id = parseInt(idStr);
    if (type === 'group') {
      await metricGroupsApi.update(id, { name: editingValue.trim() }).catch(() => {});
    } else {
      await metricsApi.update(id, { name: editingValue.trim() }).catch(() => {});
    }
    setEditingId(null);
    await fetchData().catch(() => {});
  };

  // Drag-drop handlers
  const handleDragStart = (e: React.DragEvent, metricId: number) => {
    e.dataTransfer.setData('metricId', String(metricId));
    e.dataTransfer.effectAllowed = 'move';
  };

  const handleDragOver = (e: React.DragEvent, groupId: number | null) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    setDragOverGroup(groupId);
  };

  const handleDragLeave = () => { setDragOverGroup(undefined); };

  const handleDrop = async (e: React.DragEvent, groupId: number | null) => {
    e.preventDefault();
    setDragOverGroup(undefined);
    const metricId = parseInt(e.dataTransfer.getData('metricId'));
    if (!isNaN(metricId)) {
      await metricsApi.move(metricId, groupId).catch(() => {});
      await fetchData().catch(() => {});
    }
  };

  const grouped = new Map<number | null, MetricPool[]>();
  for (const g of groups) grouped.set(g.id, []);
  grouped.set(null, []);
  for (const m of metrics) {
    const key = m.group_id ?? null;
    if (!grouped.has(key)) grouped.set(key, []);
    grouped.get(key)!.push(m);
  }

  const renderEditableText = (key: string, value: string, className: string) => {
    if (editingId === key) {
      return (
        <input
          value={editingValue}
          onChange={(e) => setEditingValue(e.target.value)}
          onBlur={saveEdit}
          onKeyDown={(e) => { if (e.key === 'Enter') saveEdit(); if (e.key === 'Escape') setEditingId(null); }}
          onClick={(e) => e.stopPropagation()}
          autoFocus
          className={`bg-obsidian-800 border border-amber-500/50 rounded px-1 py-0 min-w-0 focus:outline-none ${className}`}
        />
      );
    }
    return (
      <span
        onDoubleClick={(e) => { e.stopPropagation(); startEdit(key, value); }}
        className={`truncate ${className}`}
        title="Double-click to rename"
      >
        {value}
      </span>
    );
  };

  return (
    <>
      <div className="sticky top-0 z-10 bg-obsidian-900 -mx-2.5 px-2.5 pt-2.5">
        <div className="flex items-center justify-between mb-1.5">
          <span className="text-amber-500 text-[10px] font-bold tracking-wide uppercase">{t('nav.metrics')}</span>
          <span className="text-[9px] text-gray-600">{metrics.length}</span>
        </div>

        {/* Search */}
        <div className="relative mb-2">
          <MagnifyingGlass size={11} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-600" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t('metrics.searchPlaceholder') || '搜索指标...'}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-md pl-6 pr-2 py-1 text-[10px] text-gray-300 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
          />
          {search && (
            <button onClick={() => setSearch('')} className="absolute right-1.5 top-1/2 -translate-y-1/2 text-gray-600 hover:text-gray-400">
              <X size={10} />
            </button>
          )}
        </div>
      </div>

      {/* Search results (flat list) */}
      {search.trim() && (
        <div className="space-y-0.5 mb-2">
          {metrics
            .filter((m) => m.name.toLowerCase().includes(search.toLowerCase()))
            .map((m) => (
              <div
                key={m.id}
                onClick={() => { navigate(`/metrics?id=${m.id}`); setSearch(''); }}
                className={`flex items-center gap-1 text-[10px] px-2 py-1.5 rounded transition-premium cursor-pointer ${
                  activeMetricId === m.id
                    ? 'bg-amber-500/10 text-amber-500'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'
                }`}
              >
                <Star size={9} className="text-amber-500/60 flex-shrink-0" weight="fill" />
                <span className="truncate flex-1">{m.name}</span>
                <span className="text-[8px] text-gray-600 flex-shrink-0">
                  {groups.find((g) => g.id === m.group_id)?.name || ''}
                </span>
              </div>
            ))}
          {metrics.filter((m) => m.name.toLowerCase().includes(search.toLowerCase())).length === 0 && (
            <p className="text-[9px] text-gray-600 italic px-1">无匹配结果</p>
          )}
        </div>
      )}

      {/* Grouped tree (hidden when searching) */}
      {!search.trim() && groups.map((group) => {
        const items = grouped.get(group.id) || [];
        const isCollapsed = collapsed.has(group.id);
        const isDragOver = dragOverGroup === group.id;
        return (
          <div
            key={group.id}
            className={`mb-1 rounded transition-all ${isDragOver ? 'ring-1 ring-amber-500/30 bg-amber-500/5' : ''}`}
            onDragOver={(e) => handleDragOver(e, group.id)}
            onDragLeave={handleDragLeave}
            onDrop={(e) => handleDrop(e, group.id)}
          >
            <div
              className="flex items-center gap-1 px-1 py-1 rounded hover:bg-obsidian-800 cursor-pointer transition-premium"
              onClick={() => toggle(group.id)}
            >
              {isCollapsed ? <CaretRight size={10} className="text-gray-600" /> : <CaretDown size={10} className="text-gray-600" />}
              <Folder size={11} className={isDragOver ? 'text-amber-500' : 'text-amber-500/60'} />
              {renderEditableText(`group-${group.id}`, group.name, 'text-[10px] text-gray-300 font-medium flex-1')}
              <span className="text-[8px] text-gray-600">{items.length}</span>
            </div>
            {!isCollapsed && items.map((m) => (
              <div
                key={m.id}
                draggable
                onDragStart={(e) => handleDragStart(e, m.id)}
                onClick={() => navigate(`/metrics?id=${m.id}`)}
                className={`flex items-center gap-1 text-[10px] pl-6 pr-1.5 py-1 rounded transition-premium cursor-grab active:cursor-grabbing ${
                  activeMetricId === m.id
                    ? 'bg-amber-500/10 text-amber-500'
                    : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'
                }`}
              >
                <Star size={9} className={activeMetricId === m.id ? 'text-amber-500 flex-shrink-0' : 'text-amber-500/60 flex-shrink-0'} weight="fill" />
                {renderEditableText(`metric-${m.id}`, m.name, `text-[10px] flex-1 ${activeMetricId === m.id ? 'text-amber-500' : 'text-gray-400'}`)}
                {m.created_at && (
                  <span className="text-[8px] text-gray-600 flex-shrink-0" title={new Date(m.created_at).toLocaleString()}>
                    {new Date(m.created_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                  </span>
                )}
              </div>
            ))}
          </div>
        );
      })}

      {!search.trim() && (grouped.get(null) || []).length > 0 && (
        <div
          className={`mb-1 rounded transition-all ${dragOverGroup === null ? 'ring-1 ring-amber-500/30 bg-amber-500/5' : ''}`}
          onDragOver={(e) => handleDragOver(e, null)}
          onDragLeave={handleDragLeave}
          onDrop={(e) => handleDrop(e, null)}
        >
          <div
            className="flex items-center gap-1 px-1 py-1 rounded hover:bg-obsidian-800 cursor-pointer transition-premium"
            onClick={() => toggle(null)}
          >
            {collapsed.has(null) ? <CaretRight size={10} className="text-gray-600" /> : <CaretDown size={10} className="text-gray-600" />}
            <span className="text-[10px] text-gray-500 flex-1">{t('metrics.ungrouped')}</span>
            <span className="text-[8px] text-gray-600">{(grouped.get(null) || []).length}</span>
          </div>
          {!collapsed.has(null) && (grouped.get(null) || []).map((m) => (
            <div
              key={m.id}
              draggable
              onDragStart={(e) => handleDragStart(e, m.id)}
              onClick={() => navigate(`/metrics?id=${m.id}`)}
              className={`flex items-center gap-1 text-[10px] pl-5 pr-1.5 py-1 rounded transition-premium cursor-grab active:cursor-grabbing ${
                activeMetricId === m.id
                  ? 'bg-amber-500/10 text-amber-500'
                  : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'
              }`}
            >
              <Star size={9} className={activeMetricId === m.id ? 'text-amber-500 flex-shrink-0' : 'text-amber-500/60 flex-shrink-0'} weight="fill" />
              {renderEditableText(`metric-${m.id}`, m.name, `text-[10px] flex-1 ${activeMetricId === m.id ? 'text-amber-500' : 'text-gray-400'}`)}
              {m.created_at && (
                <span className="text-[8px] text-gray-600 flex-shrink-0" title={new Date(m.created_at).toLocaleString()}>
                  {new Date(m.created_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                </span>
              )}            </div>
          ))}
        </div>
      )}

      {!search.trim() && metrics.length === 0 && <p className="text-[9px] text-gray-600 italic px-1">{t('metrics.empty.sidebar')}</p>}
    </>
  );
}

// ── Conversations Panel (minimal — sidebar is in the page itself) ──
function ConversationsPanel({ t }: { t: (k: string) => string }) {
  return (
    <div className="flex items-center justify-center h-full">
      <p className="text-[9px] text-gray-600 text-center px-2">{t('assetPanel.convHint')}</p>
    </div>
  );
}
