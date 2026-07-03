import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams, useNavigate } from 'react-router-dom';
import {
  Plus,
  Plug,
  ArrowClockwise,
  Trash,
  X,
  Lightning,
  MagnifyingGlass,
  Graph as GraphIcon,
  Terminal,
  Play,
  PencilSimple,
  Key,
} from '@phosphor-icons/react';
import {
  datasourcesApi,
  knowledgeGraphApi,
  queryApi,
  tableDescriptionsApi,
  columnDescriptionsApi,
  usersApi,
  type CreateDataSourcePayload,
  type KnowledgeGraph,
  type GraphNode,
  type GraphEdge,
  type ColumnInfo,
  type QueryResult,
  type User,
} from '../lib/api';
import { PageHeader, ErrorBanner, EmptyState, StatusDot, ConfirmDialog } from '../components/ui';
import KnowledgeBasePanel from '../components/KnowledgeBasePanel';
import { useDataSourceStore } from '../stores/datasourceStore';
import { isAdmin } from '../lib/currentUser';

// ── Constants ──

const EMPTY_FORM: CreateDataSourcePayload = {
  name: '',
  db_type: 'mysql',
  host: '127.0.0.1',
  port: 3306,
  database_name: '',
  username: 'root',
  password: '',
};

const DEFAULT_PORTS: Record<string, number> = {
  mysql: 3306,
  postgresql: 5432,
  oracle: 1521,
};

// ── SQL Editor with Autocomplete ──

const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'IN', 'LIKE', 'BETWEEN',
  'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER', 'ON', 'AS', 'GROUP', 'BY',
  'ORDER', 'ASC', 'DESC', 'LIMIT', 'OFFSET', 'HAVING', 'DISTINCT', 'COUNT',
  'SUM', 'AVG', 'MIN', 'MAX', 'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET',
  'DELETE', 'CREATE', 'TABLE', 'ALTER', 'DROP', 'INDEX', 'UNION', 'ALL',
  'EXISTS', 'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'NULL', 'IS', 'COALESCE',
];

function formatSql(sql: string): string {
  // Normalize whitespace
  let s = sql.replace(/\s+/g, ' ').trim();

  // Uppercase keywords
  for (const kw of SQL_KEYWORDS) {
    const regex = new RegExp(`\\b${kw}\\b`, 'gi');
    s = s.replace(regex, kw);
  }

  // Handle compound keywords first
  const compounds = ['LEFT JOIN', 'RIGHT JOIN', 'INNER JOIN', 'OUTER JOIN', 'CROSS JOIN', 'GROUP BY', 'ORDER BY', 'UNION ALL'];
  for (const ck of compounds) {
    const regex = new RegExp(`\\b${ck}\\b`, 'gi');
    s = s.replace(regex, `\n${ck}`);
  }

  // Add newlines before single keywords
  const singles = ['SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'JOIN', 'HAVING', 'LIMIT', 'OFFSET', 'UNION', 'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET', 'DELETE', 'ON'];
  for (const kw of singles) {
    // Don't double-break compounds already handled
    const regex = new RegExp(`(?<!\n)(?<!\\w)\\b${kw}\\b(?! JOIN| BY| ALL)`, 'g');
    s = s.replace(regex, `\n${kw}`);
  }

  // Indent lines after SELECT, SET
  const lines = s.split('\n').map((l) => l.trim()).filter((l) => l);
  const result: string[] = [];
  for (const line of lines) {
    const upper = line.trimStart().toUpperCase();
    if (upper.startsWith('AND ') || upper.startsWith('OR ')) {
      result.push('  ' + line);
    } else if (upper.startsWith('ON ')) {
      result.push('    ' + line);
    } else {
      result.push(line);
    }
  }

  return result.join('\n');
}

interface SqlEditorProps {
  value: string;
  onChange: (v: string) => void;
  onRun: () => void;
  graph: KnowledgeGraph | null;
  placeholder?: string;
}

function SqlEditor({ value, onChange, onRun, graph, placeholder }: SqlEditorProps) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const mirrorRef = useRef<HTMLDivElement>(null);
  const [suggestions, setSuggestions] = useState<{ label: string; type: 'table' | 'column' | 'keyword' }[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [cursorWord, setCursorWord] = useState('');
  const [dropdownPos, setDropdownPos] = useState({ top: 0, left: 0 });

  // Build base suggestion list from graph + SQL keywords
  const allSuggestions = useMemo(() => {
    const items: { label: string; type: 'table' | 'column' | 'keyword' }[] = [];
    if (graph) {
      for (const node of graph.nodes) {
        items.push({ label: node.label, type: 'table' });
        for (const col of node.columns) {
          items.push({ label: col.name, type: 'column' });
        }
      }
    }
    for (const kw of SQL_KEYWORDS) {
      items.push({ label: kw, type: 'keyword' });
    }
    const seen = new Set<string>();
    return items.filter((item) => {
      const key = item.label.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  }, [graph]);

  // Parse table aliases from SQL text: "FROM table alias" or "JOIN table alias" or "table AS alias"
  const parseAliases = (sql: string): Map<string, string> => {
    const aliases = new Map<string, string>(); // alias → tableName
    const pattern = /(?:FROM|JOIN)\s+(\w+)\s+(?:AS\s+)?(\w+)/gi;
    let match;
    while ((match = pattern.exec(sql)) !== null) {
      const tableName = match[1];
      const alias = match[2];
      // Skip SQL keywords as aliases
      if (!SQL_KEYWORDS.includes(alias.toUpperCase())) {
        aliases.set(alias.toLowerCase(), tableName.toLowerCase());
      }
    }
    return aliases;
  };

  const getWordAtCursor = (text: string, pos: number): string => {
    const before = text.slice(0, pos);
    const match = before.match(/[\w.]+$/);
    return match ? match[0] : '';
  };

  // Calculate cursor pixel position for dropdown placement
  const getCursorPosition = () => {
    const textarea = textareaRef.current;
    const mirror = mirrorRef.current;
    if (!textarea || !mirror) return { top: 24, left: 12 };

    const pos = textarea.selectionStart;
    const textBefore = value.slice(0, pos);

    // Mirror the text up to cursor to measure position
    mirror.textContent = textBefore;
    mirror.style.width = `${textarea.clientWidth}px`;

    const span = document.createElement('span');
    span.textContent = '|';
    mirror.appendChild(span);

    const top = span.offsetTop + 20 - textarea.scrollTop;
    const left = Math.min(span.offsetLeft, textarea.clientWidth - 260);

    mirror.removeChild(span);
    return { top: Math.max(top, 20), left: Math.max(left, 0) };
  };

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const newValue = e.target.value;
    onChange(newValue);

    const pos = e.target.selectionStart;
    const word = getWordAtCursor(newValue, pos);
    setCursorWord(word);

    if (word.length >= 1) {
      const lower = word.toLowerCase();
      const dotIdx = lower.lastIndexOf('.');
      let filtered: { label: string; type: 'table' | 'column' | 'keyword' }[] = [];

      if (dotIdx >= 0) {
        const prefix = lower.slice(0, dotIdx);
        const colPrefix = lower.slice(dotIdx + 1);

        // Check if prefix is a table name directly
        let tableNode = graph?.nodes.find((n) => n.label.toLowerCase() === prefix);

        // If not, check if it's an alias
        if (!tableNode) {
          const aliases = parseAliases(newValue);
          const realTable = aliases.get(prefix);
          if (realTable) {
            tableNode = graph?.nodes.find((n) => n.label.toLowerCase() === realTable);
          }
        }

        if (tableNode) {
          filtered = tableNode.columns
            .filter((c) => c.name.toLowerCase().startsWith(colPrefix))
            .map((c) => ({ label: c.name, type: 'column' as const }));
        }
      } else {
        filtered = allSuggestions.filter((s) => s.label.toLowerCase().startsWith(lower));
      }

      setSuggestions(filtered.slice(0, 10));
      setShowSuggestions(filtered.length > 0);
      setSelectedIdx(0);

      // Update dropdown position
      requestAnimationFrame(() => {
        setDropdownPos(getCursorPosition());
      });
    } else {
      setShowSuggestions(false);
    }
  };

  const applySuggestion = (label: string) => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    const pos = textarea.selectionStart;
    const before = value.slice(0, pos);
    const after = value.slice(pos);

    const dotIdx = cursorWord.lastIndexOf('.');
    let replaceLen: number;
    if (dotIdx >= 0) {
      replaceLen = cursorWord.length - dotIdx - 1;
    } else {
      replaceLen = cursorWord.length;
    }

    const newBefore = before.slice(0, before.length - replaceLen);
    const newValue = newBefore + label + after;
    onChange(newValue);
    setShowSuggestions(false);

    requestAnimationFrame(() => {
      const newPos = newBefore.length + label.length;
      textarea.selectionStart = newPos;
      textarea.selectionEnd = newPos;
      textarea.focus();
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      onRun();
      setShowSuggestions(false);
      return;
    }

    if (showSuggestions) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIdx((prev) => Math.min(prev + 1, suggestions.length - 1));
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIdx((prev) => Math.max(prev - 1, 0));
      } else if (e.key === 'Tab' || e.key === 'Enter') {
        if (suggestions[selectedIdx]) {
          e.preventDefault();
          applySuggestion(suggestions[selectedIdx].label);
        }
      } else if (e.key === 'Escape') {
        setShowSuggestions(false);
      }
    }
  };

  const typeColors: Record<string, string> = {
    table: 'text-amber-500',
    column: 'text-blue-400',
    keyword: 'text-gray-400',
  };

  const typeLabels: Record<string, string> = {
    table: 'TBL',
    column: 'COL',
    keyword: 'SQL',
  };

  return (
    <div className="relative">
      {/* Hidden mirror div for cursor position measurement */}
      <div
        ref={mirrorRef}
        aria-hidden="true"
        className="absolute top-0 left-0 invisible overflow-hidden whitespace-pre-wrap break-words text-xs font-mono px-3 py-2 pointer-events-none"
        style={{ wordWrap: 'break-word' }}
      />
      <textarea
        ref={textareaRef}
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        onBlur={() => setTimeout(() => setShowSuggestions(false), 150)}
        placeholder={placeholder}
        className="w-full bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-data-green font-mono focus:outline-none focus:border-amber-500/50 transition-premium resize-y min-h-[60px] max-h-[300px]"
        rows={3}
        spellCheck={false}
        autoComplete="off"
      />
      {/* Autocomplete dropdown - positioned near cursor */}
      {showSuggestions && suggestions.length > 0 && (
        <div
          className="absolute z-30 bg-obsidian-900 border border-obsidian-700 rounded-lg shadow-2xl overflow-hidden max-h-[180px] overflow-y-auto scrollbar-thin w-64"
          style={{ top: `${dropdownPos.top}px`, left: `${dropdownPos.left}px` }}
        >
          {suggestions.map((s, i) => (
            <div
              key={`${s.label}-${s.type}-${i}`}
              onMouseDown={(e) => { e.preventDefault(); applySuggestion(s.label); }}
              className={`flex items-center gap-2 px-2.5 py-1.5 cursor-pointer transition-colors ${
                i === selectedIdx ? 'bg-obsidian-800' : 'hover:bg-obsidian-800/50'
              }`}
            >
              <span className={`text-[8px] font-bold font-mono w-6 ${typeColors[s.type]}`}>
                {typeLabels[s.type]}
              </span>
              <span className="text-[10px] font-mono text-gray-200 flex-1 truncate">{s.label}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── ER Diagram ──

const TABLE_HEADER_H = 32;
const COL_ROW_H = 20;
const TABLE_W = 180;
const TABLE_PAD_X = 60;
const TABLE_PAD_Y = 40;
const COLS_PER_ROW = 4; // tables per row in grid layout
const MAX_VISIBLE_COLS = 4; // only show first N columns in compact view

interface TableRect {
  id: string;
  label: string;
  columns: ColumnInfo[];
  x: number;
  y: number;
  w: number;
  h: number;
}

function layoutTables(nodes: GraphNode[]): TableRect[] {
  const rects: TableRect[] = [];
  let col = 0;
  let maxRowH = 0;
  let yOffset = TABLE_PAD_Y;

  for (const node of nodes) {
    const visibleCols = Math.min(node.columns.length, MAX_VISIBLE_COLS);
    const hasMore = node.columns.length > MAX_VISIBLE_COLS;
    const h = TABLE_HEADER_H + visibleCols * COL_ROW_H + (hasMore ? 18 : 0) + 4;
    const x = TABLE_PAD_X + col * (TABLE_W + TABLE_PAD_X);
    const y = yOffset;
    rects.push({ id: node.id, label: node.label, columns: node.columns, x, y, w: TABLE_W, h });
    maxRowH = Math.max(maxRowH, h);
    col++;
    if (col >= COLS_PER_ROW) {
      col = 0;
      yOffset += maxRowH + TABLE_PAD_Y;
      maxRowH = 0;
    }
  }
  return rects;
}

function getAnchorPoints(rect: TableRect, side: 'left' | 'right') {
  const x = side === 'left' ? rect.x : rect.x + rect.w;
  const y = rect.y + rect.h / 2;
  return { x, y };
}

// ── Editable column row (shows DB comment + user-editable business note) ──
function ColumnRow({ col, description, onSave, editable }: {
  col: ColumnInfo;
  description: string;
  onSave: (value: string) => void;
  editable: boolean;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');

  const startEdit = () => {
    if (!editable) return;
    setDraft(description);
    setEditing(true);
  };

  const commit = () => {
    onSave(draft);
    setEditing(false);
  };

  // What to show as the column's note: user description takes priority over DB comment
  const note = description || col.comment || '';

  return (
    <div className="px-2 py-1.5 rounded hover:bg-obsidian-800 transition-premium group">
      <div className="flex items-center gap-1.5">
        <span className="w-5 text-[8px] font-mono font-bold flex-shrink-0 text-center">
          {col.is_primary_key ? <span className="text-amber-500">PK</span> : col.is_foreign_key ? <span className="text-blue-400">FK</span> : null}
        </span>
        <span className={`text-[10px] font-mono flex-1 truncate ${col.is_primary_key ? 'text-amber-500' : col.is_foreign_key ? 'text-blue-400' : 'text-gray-200'}`}>
          {col.name}
        </span>
        <span className="text-[9px] font-mono text-gray-500 flex-shrink-0">{col.data_type}</span>
        {!col.nullable && <span className="text-[7px] text-red-400/80 flex-shrink-0">NN</span>}
        {editable && !editing && (
          <button
            onClick={startEdit}
            className="text-gray-600 hover:text-amber-500 opacity-0 group-hover:opacity-100 transition-premium flex-shrink-0"
            title={note ? t('common.open') : t('kg.addDesc')}
          >
            <PencilSimple size={9} />
          </button>
        )}
      </div>

      {editing ? (
        <div className="mt-1 pl-6">
          <input
            type="text"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') commit(); if (e.key === 'Escape') setEditing(false); }}
            onBlur={commit}
            autoFocus
            placeholder={t('kg.colDescPlaceholder')}
            className="w-full bg-obsidian-950 border border-amber-500/40 rounded px-2 py-1 text-[9px] text-gray-200 placeholder-gray-600 focus:outline-none"
          />
        </div>
      ) : note ? (
        <p className="mt-0.5 pl-6 text-[9px] text-gray-500 leading-snug break-words">
          {description ? note : <span className="text-gray-600 italic">{note}</span>}
        </p>
      ) : null}
    </div>
  );
}

// ── Table Detail Panel ──

function TableDetailPanel({ table, edges, datasourceId, onClose }: {
  table: GraphNode;
  edges: GraphEdge[];
  datasourceId: number | null;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const admin = isAdmin();
  const relatedEdges = edges.filter((e) => e.source === table.id || e.target === table.id);

  const [description, setDescription] = useState('');
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState('');
  const [saving, setSaving] = useState(false);

  // Column descriptions keyed by column name
  const [colDescs, setColDescs] = useState<Record<string, string>>({});

  // Load the saved description for this table
  useEffect(() => {
    setEditing(false);
    setDescription('');
    setColDescs({});
    if (datasourceId == null) return;
    tableDescriptionsApi.list(datasourceId)
      .then((items) => {
        const found = items.find((d) => d.table_name === table.id || d.table_name === table.label);
        setDescription(found?.description || '');
      })
      .catch(() => {});
    columnDescriptionsApi.list(datasourceId)
      .then((items) => {
        const map: Record<string, string> = {};
        for (const it of items) {
          if (it.table_name === table.label || it.table_name === table.id) {
            map[it.column_name] = it.description;
          }
        }
        setColDescs(map);
      })
      .catch(() => {});
  }, [datasourceId, table.id, table.label]);

  const handleSave = async () => {
    if (datasourceId == null) return;
    setSaving(true);
    try {
      await tableDescriptionsApi.upsert(datasourceId, table.label, draft.trim());
      setDescription(draft.trim());
      setEditing(false);
    } catch {
      /* silent */
    } finally {
      setSaving(false);
    }
  };

  const handleSaveColumn = async (columnName: string, value: string) => {
    if (datasourceId == null) return;
    try {
      await columnDescriptionsApi.upsert(datasourceId, table.label, columnName, value.trim());
      setColDescs((prev) => {
        const next = { ...prev };
        if (value.trim()) next[columnName] = value.trim();
        else delete next[columnName];
        return next;
      });
    } catch {
      /* silent */
    }
  };

  return (
    <div className="absolute right-0 top-0 h-full w-72 bg-obsidian-900 border-l border-obsidian-700 z-20 flex flex-col shadow-2xl">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-obsidian-700 flex-shrink-0">
        <div className="flex items-center gap-2">
          <div className="w-2 h-2 rounded-full bg-amber-500" />
          <h3 className="text-xs font-bold text-gray-100 font-mono">{table.label}</h3>
          <span className="text-[9px] text-gray-500">{table.columns.length} cols</span>
        </div>
        <button onClick={onClose} className="text-gray-500 hover:text-gray-300 transition-premium">
          <X size={14} />
        </button>
      </div>

      <div className="flex-1 overflow-auto scrollbar-thin px-3 py-3">
        {/* Table description — business notes for AI SQL generation */}
        <div className="mb-4">
          <div className="flex items-center justify-between mb-1.5 px-1">
            <span className="text-[9px] text-gray-500 uppercase tracking-wider">{t('kg.tableDesc')}</span>
            {admin && !editing && (
              <button
                onClick={() => { setDraft(description); setEditing(true); }}
                className="text-[9px] text-amber-500 hover:text-amber-400 flex items-center gap-0.5 transition-premium"
              >
                <PencilSimple size={9} />
                {description ? t('common.open') : t('kg.addDesc')}
              </button>
            )}
          </div>
          {editing ? (
            <div>
              <textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                rows={4}
                autoFocus
                placeholder={t('kg.tableDescPlaceholder')}
                className="w-full bg-obsidian-950 border border-amber-500/40 rounded-lg px-2.5 py-2 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none resize-y"
              />
              <div className="flex items-center gap-2 mt-1.5">
                <button
                  onClick={handleSave}
                  disabled={saving}
                  className="text-[9px] text-[#08080c] font-medium bg-amber-500 hover:bg-amber-400 px-2.5 py-1 rounded transition-premium disabled:opacity-50"
                >
                  {saving ? '...' : t('common.save')}
                </button>
                <button
                  onClick={() => setEditing(false)}
                  className="text-[9px] text-gray-500 hover:text-gray-300 transition-premium"
                >
                  {t('common.cancel')}
                </button>
              </div>
            </div>
          ) : description ? (
            <p className="text-[10px] text-gray-300 leading-relaxed bg-obsidian-800/50 rounded-lg px-2.5 py-2 whitespace-pre-wrap">
              {description}
            </p>
          ) : (
            <p className="text-[9px] text-gray-600 italic px-1">{t('kg.noDesc')}</p>
          )}
        </div>

        {/* Columns list */}
        <div className="text-[9px] text-gray-500 uppercase tracking-wider mb-2 px-1">Columns</div>
        <div className="space-y-px">
          {table.columns.map((col, i) => (
            <ColumnRow
              key={i}
              col={col}
              description={colDescs[col.name] || ''}
              onSave={(val) => handleSaveColumn(col.name, val)}
              editable={datasourceId != null && admin}
            />
          ))}
        </div>

        {/* Relationships */}
        {relatedEdges.length > 0 && (
          <div className="mt-4">
            <div className="text-[9px] text-gray-500 uppercase tracking-wider mb-2 px-1">
              Relationships ({relatedEdges.length})
            </div>
            <div className="space-y-1">
              {relatedEdges.map((edge, i) => (
                <div key={i} className="flex items-center gap-1.5 px-2 py-1.5 rounded bg-obsidian-800/50 text-[9px] font-mono">
                  <span className="text-gray-300">{edge.source}</span>
                  <span className="text-amber-500">→</span>
                  <span className="text-gray-300">{edge.target}</span>
                  <span className="text-gray-600 ml-auto">{edge.on}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function ERDiagram({ graph, datasourceId }: { graph: KnowledgeGraph; datasourceId: number | null }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [transform, setTransform] = useState({ x: 0, y: 0, scale: 1 });
  const [dragging, setDragging] = useState(false);
  const [dragStart, setDragStart] = useState({ x: 0, y: 0 });
  const [hoveredTable, setHoveredTable] = useState<string | null>(null);
  const [selectedTable, setSelectedTable] = useState<string | null>(null);

  const tables = useMemo(() => layoutTables(graph.nodes), [graph.nodes]);

  // Build a lookup for edges
  const edgeLines = useMemo(() => {
    const tableMap = new Map(tables.map((t) => [t.id, t]));
    return graph.edges.map((edge) => {
      const src = tableMap.get(edge.source);
      const tgt = tableMap.get(edge.target);
      if (!src || !tgt) return null;
      // Determine which side to connect from
      const srcRight = src.x + src.w;
      const tgtLeft = tgt.x;
      let srcAnchor, tgtAnchor;
      if (srcRight <= tgtLeft) {
        srcAnchor = getAnchorPoints(src, 'right');
        tgtAnchor = getAnchorPoints(tgt, 'left');
      } else if (tgt.x + tgt.w <= src.x) {
        srcAnchor = getAnchorPoints(src, 'left');
        tgtAnchor = getAnchorPoints(tgt, 'right');
      } else {
        srcAnchor = getAnchorPoints(src, 'right');
        tgtAnchor = getAnchorPoints(tgt, 'right');
      }
      return { edge, srcAnchor, tgtAnchor };
    }).filter(Boolean) as { edge: GraphEdge; srcAnchor: { x: number; y: number }; tgtAnchor: { x: number; y: number } }[];
  }, [tables, graph.edges]);

  // Pan handlers
  const handleMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    setDragging(true);
    setDragStart({ x: e.clientX - transform.x, y: e.clientY - transform.y });
  };
  const handleMouseMove = (e: React.MouseEvent) => {
    if (!dragging) return;
    setTransform((prev) => ({ ...prev, x: e.clientX - dragStart.x, y: e.clientY - dragStart.y }));
  };
  const handleMouseUp = () => setDragging(false);
  const handleWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    setTransform((prev) => ({
      ...prev,
      scale: Math.min(3, Math.max(0.2, prev.scale * delta)),
    }));
  };

  const handleTableClick = (tableId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    setSelectedTable((prev) => (prev === tableId ? null : tableId));
  };

  const selectedNode = selectedTable ? graph.nodes.find((n) => n.id === selectedTable) : null;

  return (
    <div className="relative w-full h-full">
      <div
        ref={containerRef}
        className="w-full h-full overflow-hidden cursor-grab active:cursor-grabbing"
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
        onWheel={handleWheel}
      >
      <svg
        width="100%"
        height="100%"
        className="select-none"
      >
        <defs>
          <marker id="er-arrow" viewBox="0 0 10 10" refX="10" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M 0 0 L 10 5 L 0 10 z" fill="#6b7280" />
          </marker>
          <marker id="er-diamond" viewBox="0 0 12 12" refX="6" refY="6" markerWidth="8" markerHeight="8" orient="auto">
            <path d="M 6 0 L 12 6 L 6 12 L 0 6 z" fill="#d4a853" stroke="#d4a853" strokeWidth="1" />
          </marker>
        </defs>
        <g transform={`translate(${transform.x}, ${transform.y}) scale(${transform.scale})`}>
          {/* Relationship lines */}
          {edgeLines.map(({ edge, srcAnchor, tgtAnchor }, i) => {
            const midX = (srcAnchor.x + tgtAnchor.x) / 2;
            const path = `M ${srcAnchor.x} ${srcAnchor.y} C ${midX} ${srcAnchor.y}, ${midX} ${tgtAnchor.y}, ${tgtAnchor.x} ${tgtAnchor.y}`;
            const isHighlighted = hoveredTable === edge.source || hoveredTable === edge.target;
            return (
              <g key={`edge-${i}`}>
                <path
                  d={path}
                  fill="none"
                  stroke={isHighlighted ? '#d4a853' : '#4a4a5a'}
                  strokeWidth={isHighlighted ? 2 : 1.5}
                  strokeDasharray={edge.type === 'fk' ? 'none' : '4 2'}
                  markerEnd="url(#er-arrow)"
                  className="transition-colors duration-150"
                />
                {/* Edge label */}
                <text
                  x={midX}
                  y={(srcAnchor.y + tgtAnchor.y) / 2 - 6}
                  textAnchor="middle"
                  fill={isHighlighted ? '#d4a853' : '#6b7280'}
                  fontSize="9"
                  fontFamily="monospace"
                  className="transition-colors duration-150"
                >
                  {edge.on}
                </text>
              </g>
            );
          })}

          {/* Table cards */}
          {tables.map((table) => {
            const isHighlighted = hoveredTable === table.id;
            const isSelected = selectedTable === table.id;
            const visibleCols = table.columns.slice(0, MAX_VISIBLE_COLS);
            const hiddenCount = table.columns.length - MAX_VISIBLE_COLS;
            return (
              <g
                key={table.id}
                onMouseEnter={() => setHoveredTable(table.id)}
                onMouseLeave={() => setHoveredTable(null)}
                onClick={(e) => handleTableClick(table.id, e)}
                className="cursor-pointer"
              >
                {/* Card shadow */}
                <rect
                  x={table.x + 2}
                  y={table.y + 2}
                  width={table.w}
                  height={table.h}
                  rx={8}
                  fill="#000"
                  opacity={0.3}
                />
                {/* Card body */}
                <rect
                  x={table.x}
                  y={table.y}
                  width={table.w}
                  height={table.h}
                  rx={8}
                  fill="#12121a"
                  stroke={isSelected ? '#d4a853' : isHighlighted ? '#3a3a4a' : '#2a2a3a'}
                  strokeWidth={isSelected ? 2.5 : isHighlighted ? 1.5 : 1}
                  className="transition-all duration-150"
                />
                {/* Header background */}
                <rect
                  x={table.x}
                  y={table.y}
                  width={table.w}
                  height={TABLE_HEADER_H}
                  rx={8}
                  fill={isSelected ? '#d4a853' : isHighlighted ? '#2a2a3a' : '#1e1e2e'}
                />
                <rect
                  x={table.x}
                  y={table.y + TABLE_HEADER_H - 8}
                  width={table.w}
                  height={8}
                  fill={isSelected ? '#d4a853' : isHighlighted ? '#2a2a3a' : '#1e1e2e'}
                />
                <line
                  x1={table.x}
                  y1={table.y + TABLE_HEADER_H}
                  x2={table.x + table.w}
                  y2={table.y + TABLE_HEADER_H}
                  stroke="#2a2a3a"
                  strokeWidth={1}
                />
                {/* Table name */}
                <text
                  x={table.x + 12}
                  y={table.y + TABLE_HEADER_H / 2 + 1}
                  dominantBaseline="middle"
                  fill={isSelected ? '#08080c' : '#e5e7eb'}
                  fontSize="11"
                  fontFamily="monospace"
                  fontWeight="600"
                >
                  {table.label}
                </text>
                {/* Column count */}
                <text
                  x={table.x + table.w - 12}
                  y={table.y + TABLE_HEADER_H / 2 + 1}
                  dominantBaseline="middle"
                  textAnchor="end"
                  fill={isSelected ? '#08080c' : '#6b7280'}
                  fontSize="9"
                  fontFamily="monospace"
                >
                  {table.columns.length}
                </text>
                {/* Compact columns */}
                {visibleCols.map((col, ci) => {
                  const cy = table.y + TABLE_HEADER_H + ci * COL_ROW_H + COL_ROW_H / 2;
                  return (
                    <g key={ci}>
                      {col.is_primary_key && (
                        <text x={table.x + 8} y={cy + 1} dominantBaseline="middle" fill="#d4a853" fontSize="8" fontFamily="monospace">PK</text>
                      )}
                      {col.is_foreign_key && !col.is_primary_key && (
                        <text x={table.x + 8} y={cy + 1} dominantBaseline="middle" fill="#60a5fa" fontSize="8" fontFamily="monospace">FK</text>
                      )}
                      <text
                        x={table.x + 28}
                        y={cy + 1}
                        dominantBaseline="middle"
                        fill={col.is_primary_key ? '#d4a853' : col.is_foreign_key ? '#60a5fa' : '#d1d5db'}
                        fontSize="9"
                        fontFamily="monospace"
                      >
                        {col.name.length > 12 ? col.name.slice(0, 11) + '…' : col.name}
                      </text>
                      <text
                        x={table.x + table.w - 8}
                        y={cy + 1}
                        dominantBaseline="middle"
                        textAnchor="end"
                        fill="#4b5563"
                        fontSize="8"
                        fontFamily="monospace"
                      >
                        {col.data_type.length > 8 ? col.data_type.slice(0, 7) + '…' : col.data_type}
                      </text>
                    </g>
                  );
                })}
                {/* "+N more" indicator */}
                {hiddenCount > 0 && (
                  <text
                    x={table.x + table.w / 2}
                    y={table.y + TABLE_HEADER_H + MAX_VISIBLE_COLS * COL_ROW_H + 10}
                    dominantBaseline="middle"
                    textAnchor="middle"
                    fill="#6b7280"
                    fontSize="9"
                    fontFamily="monospace"
                  >
                    +{hiddenCount} more
                  </text>
                )}
              </g>
            );
          })}
        </g>
      </svg>
    </div>

    {/* Detail Panel */}
    {selectedNode && (
      <TableDetailPanel
        table={selectedNode}
        edges={graph.edges}
        datasourceId={datasourceId}
        onClose={() => setSelectedTable(null)}
      />
    )}
    </div>
  );
}

// ── Main Page ──

export default function DataSourcesPage() {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const { sources, setSources } = useDataSourceStore();
  const [, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Form state
  const [showForm, setShowForm] = useState(false);
  const [form, setForm] = useState<CreateDataSourcePayload>(EMPTY_FORM);
  const [submitting, setSubmitting] = useState(false);

  // Selected datasource
  const dsIdParam = searchParams.get('ds');
  const selectedDsId = dsIdParam ? parseInt(dsIdParam) : null;
  const selectedDs = sources.find((s) => s.id === selectedDsId) || null;

  const admin = isAdmin();

  // Action states
  const [testingId, setTestingId] = useState<number | null>(null);
  const [introspectingId, setIntrospectingId] = useState<number | null>(null);
  const [testResult, setTestResult] = useState<string | null>(null);
  const [showGrants, setShowGrants] = useState(false);

  // Knowledge graph
  const [graph, setGraph] = useState<KnowledgeGraph | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [graphError, setGraphError] = useState<string | null>(null);
  const [search, setSearch] = useState('');

  // SQL Query panel
  const [showSqlPanel, setShowSqlPanel] = useState(false);
  const [sqlInput, setSqlInput] = useState('');
  const [sqlRunning, setSqlRunning] = useState(false);
  const [sqlResult, setSqlResult] = useState<QueryResult | null>(null);
  const [sqlError, setSqlError] = useState<string | null>(null);
  const [sqlPanelHeight, setSqlPanelHeight] = useState(280);
  const sqlResizing = useRef(false);
  const sqlResizeStart = useRef({ y: 0, h: 0 });
  const [sqlDuration, setSqlDuration] = useState<number | null>(null);
  const [sqlPage, setSqlPage] = useState(0);
  const SQL_PAGE_SIZE = 50;
  const [showSaveSqlModal, setShowSaveSqlModal] = useState(false);
  const [saveSqlName, setSaveSqlName] = useState('');
  const [cellDetail, setCellDetail] = useState<{ column: string; value: unknown } | null>(null);

  const fetchSources = useCallback(async (showLoading = false) => {
    try {
      if (showLoading) setLoading(true);
      setError(null);
      const data = await datasourcesApi.list();
      setSources(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t, setSources]);

  useEffect(() => { fetchSources(true); }, [fetchSources]);

  // Default to the first datasource when none is selected (or the selected one
  // no longer exists), so the page isn't empty on first open.
  useEffect(() => {
    if (sources.length === 0) return;
    const valid = selectedDsId != null && sources.some((s) => s.id === selectedDsId);
    if (!valid) {
      navigate(`/datasources?ds=${sources[0].id}`, { replace: true });
    }
  }, [sources, selectedDsId, navigate]);

  // Listen for SQL favorite load events
  useEffect(() => {
    const handler = (e: Event) => {
      const sql = (e as CustomEvent).detail?.sql;
      if (sql) {
        setSqlInput(sql);
        setShowSqlPanel(true);
      }
    };
    window.addEventListener('sql-favorite-load', handler);
    return () => window.removeEventListener('sql-favorite-load', handler);
  }, []);

  // Load graph when selected datasource changes
  useEffect(() => {
    if (selectedDsId) {
      setGraphLoading(true);
      setGraphError(null);
      setTestResult(null);
      knowledgeGraphApi.get(selectedDsId)
        .then(setGraph)
        .catch(() => { setGraph(null); setGraphError(t('errors.noKnowledgeGraph')); })
        .finally(() => setGraphLoading(false));
    } else {
      setGraph(null);
      setGraphError(null);
    }
  }, [selectedDsId, t]);

  // ── Handlers ──

  const handleSubmit = async () => {
    try {
      setSubmitting(true);
      await datasourcesApi.create(form);
      setShowForm(false);
      setForm(EMPTY_FORM);
      await fetchSources();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.createFailed'));
    } finally {
      setSubmitting(false);
    }
  };

  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  const handleDelete = async () => {
    if (!selectedDsId) return;
    try {
      await datasourcesApi.remove(selectedDsId);
      navigate('/datasources', { replace: true });
      setGraph(null);
      setShowDeleteConfirm(false);
      await fetchSources();
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.deleteFailed'));
      setShowDeleteConfirm(false);
    }
  };

  const handleTest = async () => {
    if (!selectedDsId) return;
    try {
      setTestingId(selectedDsId);
      setTestResult(null);
      const result = await datasourcesApi.testConnection(selectedDsId);
      setTestResult(result.status === 'connected' ? 'connected' : result.message);
      await fetchSources();
    } catch {
      setTestResult(t('datasources.statusConnectionFailed'));
    } finally {
      setTestingId(null);
    }
  };

  const handleIntrospect = async () => {
    if (!selectedDsId) return;
    try {
      setIntrospectingId(selectedDsId);
      await datasourcesApi.introspect(selectedDsId);
      await fetchSources();
      // Refresh graph after introspection
      try {
        const data = await knowledgeGraphApi.refresh(selectedDsId);
        setGraph(data);
        setGraphError(null);
      } catch {
        // Graph refresh may fail if no relationships
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t('errors.introspectionFailed'));
    } finally {
      setIntrospectingId(null);
    }
  };

  const handleRefreshGraph = async () => {
    if (!selectedDsId) return;
    try {
      setGraphLoading(true);
      setGraphError(null);
      const data = await knowledgeGraphApi.refresh(selectedDsId);
      setGraph(data);
    } catch (e) {
      setGraphError(e instanceof Error ? e.message : t('errors.refreshFailed'));
    } finally {
      setGraphLoading(false);
    }
  };

  const handleRunSql = async () => {
    if (!selectedDsId || !sqlInput.trim()) return;
    setSqlRunning(true);
    setSqlError(null);
    setSqlResult(null);
    setSqlDuration(null);
    setSqlPage(0);
    const start = performance.now();
    try {
      const result = await queryApi.execute(sqlInput.trim(), selectedDsId);
      setSqlDuration(performance.now() - start);
      setSqlResult(result);
    } catch (e) {
      setSqlDuration(performance.now() - start);
      setSqlError(e instanceof Error ? e.message : 'Query failed');
    } finally {
      setSqlRunning(false);
    }
  };

  // Filter graph by search
  const filteredGraph = graph && search ? {
    nodes: graph.nodes.filter((n) => n.label.toLowerCase().includes(search.toLowerCase())),
    edges: graph.edges.filter((e) => {
      const ids = new Set(graph.nodes.filter((n) => n.label.toLowerCase().includes(search.toLowerCase())).map((n) => n.id));
      return ids.has(e.source) || ids.has(e.target);
    }),
  } : graph;

  // ── No datasource selected: show form or empty state ──
  if (!selectedDsId) {
    return (
      <div className="p-6">
        <PageHeader
          title={t('datasources.title')}
          description={t('datasources.description')}
        />
        {error && <ErrorBanner message={error} onDismiss={() => setError(null)} />}

        {showForm ? (
          <DataSourceForm
            t={t}
            form={form}
            setForm={setForm}
            submitting={submitting}
            onSubmit={handleSubmit}
            onCancel={() => setShowForm(false)}
          />
        ) : (
          <EmptyState
            icon={Plug}
            title={admin ? t('datasources.empty.selectOrAdd') : t('datasources.empty.noAccessTitle')}
            description={admin ? t('datasources.empty.selectOrAddDesc') : t('datasources.empty.noAccessDesc')}
            action={
              admin ? (
                <button
                  onClick={() => setShowForm(true)}
                  className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]"
                >
                  <Plus size={14} weight="bold" />
                  {t('datasources.connectDatabase')}
                </button>
              ) : undefined
            }
          />
        )}
      </div>
    );
  }

  // ── Datasource selected: show detail + knowledge graph ──
  return (
    <div className="p-4 h-full flex flex-col">
      {/* Header with datasource info + actions */}
      <div className="flex-shrink-0 mb-2">
        <PageHeader
          title={selectedDs?.name || `#${selectedDsId}`}
          description={
            selectedDs && (
              <span className="flex items-center gap-3">
                <StatusDot status={selectedDs.status} />
                <span className="font-mono">{selectedDs.host}:{selectedDs.port} / {selectedDs.database_name}</span>
                <span className="text-gray-600">({selectedDs.db_type})</span>
                {testResult && (
                  <span className={`text-[10px] px-2 py-0.5 rounded-full ${
                    testResult === 'connected' ? 'bg-data-green/10 text-data-green' : 'bg-red-500/10 text-red-400'
                  }`}>
                    {testResult === 'connected' ? t('datasources.statusConnected') : testResult}
                  </span>
                )}
              </span>
            )
          }
          action={
            <div className="flex items-center gap-2">
              {admin && (
                <button
                  onClick={handleTest}
                  disabled={testingId === selectedDsId}
                  className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium disabled:opacity-50"
                >
                  <Lightning size={12} />
                  {testingId === selectedDsId ? t('settings.testing') : t('common.test')}
                </button>
              )}
              {admin && (
                <button
                  onClick={handleIntrospect}
                  disabled={introspectingId === selectedDsId}
                  className="flex items-center gap-1 text-[10px] text-amber-500 hover:text-amber-400 bg-amber-500/5 hover:bg-amber-500/10 border border-amber-500/20 px-2.5 py-1.5 rounded-md transition-premium disabled:opacity-50"
                >
                  <MagnifyingGlass size={12} />
                  {introspectingId === selectedDsId ? t('datasources.scanning') : t('datasources.introspect')}
                </button>
              )}
              {admin && (
                <button
                  onClick={handleRefreshGraph}
                  disabled={graphLoading}
                  className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium disabled:opacity-50"
                >
                  <ArrowClockwise size={12} />
                  {t('common.refresh')}
                </button>
              )}
              {admin && (
                <button
                  onClick={() => setShowGrants(true)}
                  className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 px-2.5 py-1.5 rounded-md transition-premium"
                >
                  <Key size={12} />
                  {t('datasources.access.button')}
                </button>
              )}
              <div className="relative">
                <MagnifyingGlass size={12} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-600" />
                <input
                  type="text"
                  placeholder={t('kg.searchPlaceholder')}
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  className="bg-obsidian-800 border border-obsidian-700 rounded-md pl-7 pr-2 py-1.5 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium w-36"
                />
              </div>
              <button
                onClick={() => setShowSqlPanel(!showSqlPanel)}
                className={`flex items-center gap-1 text-[10px] px-2.5 py-1.5 rounded-md border transition-premium ${
                  showSqlPanel
                    ? 'bg-amber-500/10 border-amber-500/30 text-amber-500'
                    : 'text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border-obsidian-700'
                }`}
              >
                <Terminal size={12} />
                SQL
              </button>
              {admin && (
                <button
                  onClick={() => setShowDeleteConfirm(true)}
                  className="text-gray-600 hover:text-red-400 p-1.5 rounded transition-premium"
                  title={t('common.delete')}
                  aria-label={t('common.delete')}
                >
                  <Trash size={14} />
                </button>
              )}
            </div>
          }
        />

        {admin && showGrants && selectedDsId != null && (
          <GrantsModal
            datasourceId={selectedDsId}
            datasourceName={selectedDs?.name || `#${selectedDsId}`}
            onClose={() => setShowGrants(false)}
          />
        )}

        {/* Error banner */}
        {error && (
          <div className="mt-1">
            <ErrorBanner message={error} onDismiss={() => setError(null)} />
          </div>
        )}
      </div>

      {/* Knowledge Graph Canvas */}
      <div className={`bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden min-h-0 ${showSqlPanel ? 'flex-1 min-h-[200px]' : 'flex-1'}`}>
        {graphLoading && (
          <div className="h-full flex items-center justify-center">
            <div className="flex items-center gap-2 text-xs text-gray-500">
              <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
              {t('kg.loadingGraph')}
            </div>
          </div>
        )}

        {!graphLoading && graphError && !graph && (
          <EmptyState
            icon={GraphIcon}
            title={t('kg.empty.noGraph')}
            description={t('kg.empty.runIntrospect')}
            action={
              <button
                onClick={handleIntrospect}
                disabled={introspectingId === selectedDsId}
                className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 disabled:bg-obsidian-700 disabled:text-gray-600 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]"
              >
                <MagnifyingGlass size={14} />
                {introspectingId === selectedDsId ? t('datasources.scanning') : t('datasources.introspect')}
              </button>
            }
          />
        )}

        {!graphLoading && !graphError && filteredGraph && (
          filteredGraph.nodes.length === 0 ? (
            <div className="h-full flex items-center justify-center">
              <p className="text-xs text-gray-500">{search ? t('kg.noMatch', { search }) : t('kg.empty.noGraph')}</p>
            </div>
          ) : (
            <ERDiagram graph={filteredGraph} datasourceId={selectedDsId} />
          )
        )}
      </div>

      {/* Legend */}
      {graph && graph.nodes.length > 0 && !showSqlPanel && (
        <div className="flex items-center gap-4 mt-3 flex-shrink-0 text-[10px] text-gray-600">
          <span className="flex items-center gap-1">
            <span className="font-mono text-[9px] text-amber-500">PK</span> {t('kg.legend.table')}
          </span>
          <span className="flex items-center gap-1">
            <span className="font-mono text-[9px] text-blue-400">FK</span> {t('kg.legend.relationship')}
          </span>
          <span className="ml-auto">
            {t('kg.stats', { tables: graph.nodes.length, relationships: graph.edges.length })}
          </span>
        </div>
      )}

      {/* AI Knowledge Base Panel */}
      <div className="flex-shrink-0 mt-3">
        <KnowledgeBasePanel datasourceId={selectedDsId!} />
      </div>

      {/* SQL Query Panel */}
      {showSqlPanel && (
        <div className="flex-shrink-0 mt-3 bg-obsidian-900 border border-obsidian-700 rounded-xl overflow-hidden flex flex-col" style={{ height: `${sqlPanelHeight}px` }}>
          {/* Resize handle */}
          <div
            className="flex-shrink-0 h-2 cursor-ns-resize flex items-center justify-center hover:bg-obsidian-800 transition-colors group"
            onMouseDown={(e) => {
              e.preventDefault();
              sqlResizing.current = true;
              sqlResizeStart.current = { y: e.clientY, h: sqlPanelHeight };
              const onMove = (ev: MouseEvent) => {
                if (!sqlResizing.current) return;
                const delta = sqlResizeStart.current.y - ev.clientY;
                const newH = Math.max(150, Math.min(600, sqlResizeStart.current.h + delta));
                setSqlPanelHeight(newH);
              };
              const onUp = () => {
                sqlResizing.current = false;
                document.removeEventListener('mousemove', onMove);
                document.removeEventListener('mouseup', onUp);
              };
              document.addEventListener('mousemove', onMove);
              document.addEventListener('mouseup', onUp);
            }}
          >
            <div className="w-8 h-0.5 rounded-full bg-obsidian-700 group-hover:bg-gray-500 transition-colors" />
          </div>
          {/* SQL Panel Header */}
          <div className="flex items-center justify-between px-3 py-2 border-b border-obsidian-700 flex-shrink-0">
            <div className="flex items-center gap-2">
              <Terminal size={12} className="text-amber-500" />
              <span className="text-[10px] font-bold text-gray-300 uppercase tracking-wide">SQL Query</span>
              {selectedDs && <span className="text-[9px] text-gray-500 font-mono">→ {selectedDs.name}</span>}
            </div>
            <div className="flex items-center gap-1.5">
              <button
                onClick={() => setSqlInput(formatSql(sqlInput))}
                disabled={!sqlInput.trim()}
                className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 px-2 py-1 rounded-md transition-premium disabled:opacity-40"
              >
                Format
              </button>
              <button
                onClick={() => {
                  if (!sqlInput.trim()) return;
                  setSaveSqlName(sqlInput.trim().slice(0, 30));
                  setShowSaveSqlModal(true);
                }}
                disabled={!sqlInput.trim()}
                className="flex items-center gap-1 text-[10px] text-gray-400 hover:text-gray-200 bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 px-2 py-1 rounded-md transition-premium disabled:opacity-40"
              >
                ★ Save
              </button>
              <button
                onClick={handleRunSql}
                disabled={sqlRunning || !sqlInput.trim()}
                className="flex items-center gap-1 text-[10px] bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold px-2.5 py-1 rounded-md transition-premium disabled:opacity-40 disabled:cursor-not-allowed"
              >
                <Play size={10} weight="fill" />
                {sqlRunning ? '...' : 'Run'}
              </button>
              <button
                onClick={() => setShowSqlPanel(false)}
                className="text-gray-500 hover:text-gray-300 transition-premium"
              >
                <X size={12} />
              </button>
            </div>
          </div>

          {/* SQL Editor + Results */}
          <div className="flex-1 flex flex-col min-h-0">
            {/* SQL Input with Autocomplete */}
            <div className="flex-shrink-0 p-2 border-b border-obsidian-700">
              <SqlEditor
                value={sqlInput}
                onChange={setSqlInput}
                onRun={handleRunSql}
                graph={graph}
                placeholder="SELECT * FROM table_name LIMIT 100;  (Ctrl+Enter to run)"
              />
            </div>

            {/* Results */}
            <div className="flex-1 overflow-auto scrollbar-thin p-2 min-h-0">
              {sqlError && (
                <div className="text-[10px] text-red-400 bg-red-500/10 border border-red-500/20 rounded-md px-3 py-2 font-mono">
                  {sqlError}
                </div>
              )}
              {sqlResult && (
                <div className="text-[10px]">
                  <div className="flex items-center gap-2 mb-1.5 text-gray-500">
                    <span>{sqlResult.row_count} rows</span>
                    <span>·</span>
                    <span>{sqlResult.columns.length} columns</span>
                    {sqlDuration !== null && (
                      <>
                        <span>·</span>
                        <span>{sqlDuration < 1000 ? `${Math.round(sqlDuration)}ms` : `${(sqlDuration / 1000).toFixed(2)}s`}</span>
                      </>
                    )}
                  </div>
                  <div className="overflow-auto rounded-md border border-obsidian-700">
                    <table className="w-full text-left">
                      <thead>
                        <tr className="bg-obsidian-800">
                          {sqlResult.columns.map((col) => (
                            <th key={col} className="px-2 py-1.5 text-[9px] font-semibold text-gray-400 font-mono whitespace-nowrap border-b border-obsidian-700">
                              {col}
                            </th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {(sqlResult.rows as Record<string, unknown>[]).slice(sqlPage * SQL_PAGE_SIZE, (sqlPage + 1) * SQL_PAGE_SIZE).map((row, ri) => (
                          <tr key={ri} className="hover:bg-obsidian-800/50 border-b border-obsidian-700/30">
                            {sqlResult.columns.map((col) => {
                              const cellVal = row[col];
                              const isComplex = cellVal !== null && typeof cellVal === 'object';
                              return (
                                <td key={col} className="px-2 py-1 text-[9px] text-gray-300 font-mono whitespace-nowrap max-w-[200px] truncate">
                                  {cellVal === null ? (
                                    <span className="text-gray-600 italic">NULL</span>
                                  ) : isComplex ? (
                                    <button
                                      onClick={() => setCellDetail({ column: col, value: cellVal })}
                                      className="inline-flex items-center gap-1 text-amber-500 hover:text-amber-400 hover:underline transition-premium"
                                      title="查看详情"
                                    >
                                      <span>{Array.isArray(cellVal) ? `[${cellVal.length} 项]` : '{对象}'}</span>
                                      <span className="text-[8px]">查看详情</span>
                                    </button>
                                  ) : (
                                    String(cellVal)
                                  )}
                                </td>
                              );
                            })}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {/* Pagination */}
                  {sqlResult.row_count > SQL_PAGE_SIZE && (
                    <div className="flex items-center justify-between mt-2 px-1">
                      <span className="text-[9px] text-gray-500">
                        {sqlPage * SQL_PAGE_SIZE + 1}-{Math.min((sqlPage + 1) * SQL_PAGE_SIZE, sqlResult.row_count)} / {sqlResult.row_count}
                      </span>
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => setSqlPage(0)}
                          disabled={sqlPage === 0}
                          className="text-[9px] text-gray-400 hover:text-gray-200 px-1.5 py-0.5 rounded border border-obsidian-700 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
                        >
                          «
                        </button>
                        <button
                          onClick={() => setSqlPage((p) => Math.max(0, p - 1))}
                          disabled={sqlPage === 0}
                          className="text-[9px] text-gray-400 hover:text-gray-200 px-1.5 py-0.5 rounded border border-obsidian-700 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
                        >
                          ‹
                        </button>
                        <span className="text-[9px] text-gray-400 px-2">
                          {sqlPage + 1} / {Math.ceil(sqlResult.row_count / SQL_PAGE_SIZE)}
                        </span>
                        <button
                          onClick={() => setSqlPage((p) => Math.min(Math.ceil(sqlResult.row_count / SQL_PAGE_SIZE) - 1, p + 1))}
                          disabled={sqlPage >= Math.ceil(sqlResult.row_count / SQL_PAGE_SIZE) - 1}
                          className="text-[9px] text-gray-400 hover:text-gray-200 px-1.5 py-0.5 rounded border border-obsidian-700 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
                        >
                          ›
                        </button>
                        <button
                          onClick={() => setSqlPage(Math.ceil(sqlResult.row_count / SQL_PAGE_SIZE) - 1)}
                          disabled={sqlPage >= Math.ceil(sqlResult.row_count / SQL_PAGE_SIZE) - 1}
                          className="text-[9px] text-gray-400 hover:text-gray-200 px-1.5 py-0.5 rounded border border-obsidian-700 disabled:opacity-30 disabled:cursor-not-allowed transition-premium"
                        >
                          »
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              )}
              {!sqlResult && !sqlError && !sqlRunning && (
                <div className="h-full flex items-center justify-center text-[10px] text-gray-600">
                  Ctrl+Enter or click Run to execute
                </div>
              )}
              {sqlRunning && (
                <div className="h-full flex items-center justify-center">
                  <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* Cell Detail Modal — for JSON/object/array column values */}
      {cellDetail && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4" onClick={() => setCellDetail(null)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl w-full max-w-2xl max-h-[80vh] flex flex-col" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between px-5 py-3 border-b border-obsidian-700">
              <h3 className="text-sm font-semibold text-gray-100 font-mono">{cellDetail.column}</h3>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => {
                    navigator.clipboard.writeText(JSON.stringify(cellDetail.value, null, 2)).catch(() => {});
                  }}
                  className="text-[10px] text-gray-400 hover:text-amber-500 px-2 py-1 rounded border border-obsidian-700 transition-premium"
                >
                  复制
                </button>
                <button
                  onClick={() => setCellDetail(null)}
                  className="text-gray-500 hover:text-gray-200 transition-premium"
                >
                  <X size={16} />
                </button>
              </div>
            </div>
            <div className="overflow-auto p-4 scrollbar-thin">
              <pre className="text-[11px] text-gray-300 font-mono whitespace-pre-wrap break-words">
                {JSON.stringify(cellDetail.value, null, 2)}
              </pre>
            </div>
          </div>
        </div>
      )}

      {/* Delete Confirmation Modal */}
      <ConfirmDialog
        open={showDeleteConfirm}
        title={t('datasources.deleteConfirm.title')}
        message={t('datasources.deleteConfirm.message', { name: selectedDs?.name || '' })}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        onConfirm={handleDelete}
        onCancel={() => setShowDeleteConfirm(false)}
      />

      {/* Save SQL Favorite Modal */}
      {showSaveSqlModal && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={() => setShowSaveSqlModal(false)}>
          <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-sm font-semibold text-gray-100 mb-2">收藏 SQL</h3>
            <p className="text-xs text-gray-400 mb-3">为这条 SQL 语句命名，方便下次快速使用。</p>
            <input
              value={saveSqlName}
              onChange={(e) => setSaveSqlName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && saveSqlName.trim()) {
                  window.dispatchEvent(new CustomEvent('sql-favorite-save', { detail: { name: saveSqlName.trim(), sql: sqlInput.trim(), dsId: selectedDsId } }));
                  setShowSaveSqlModal(false);
                }
              }}
              placeholder="输入收藏名称..."
              autoFocus
              className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium mb-4"
            />
            <div className="flex items-center justify-end gap-2">
              <button
                onClick={() => setShowSaveSqlModal(false)}
                className="text-xs text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium"
              >
                {t('common.cancel')}
              </button>
              <button
                onClick={() => {
                  if (!saveSqlName.trim()) return;
                  window.dispatchEvent(new CustomEvent('sql-favorite-save', { detail: { name: saveSqlName.trim(), sql: sqlInput.trim(), dsId: selectedDsId } }));
                  setShowSaveSqlModal(false);
                }}
                disabled={!saveSqlName.trim()}
                className="text-xs text-[#08080c] bg-amber-500 hover:bg-amber-400 font-semibold px-3 py-1.5 rounded-md transition-premium disabled:opacity-40"
              >
                {t('common.save')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Extracted Form Component ──

function DataSourceForm({
  t, form, setForm, submitting, onSubmit, onCancel,
}: {
  t: (k: string) => string;
  form: CreateDataSourcePayload;
  setForm: (f: CreateDataSourcePayload) => void;
  submitting: boolean;
  onSubmit: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5">
      <div className="flex items-center justify-between mb-4">
        <h2 className="text-sm font-semibold text-gray-200">{t('datasources.newSourceTitle')}</h2>
        <button onClick={onCancel} className="text-gray-500 hover:text-gray-300"><X size={16} /></button>
      </div>
      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.name')}</label>
          <input value={form.name} onChange={(e) => setForm({ ...form, name: e.target.value })} placeholder={t('datasources.namePlaceholder')}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.databaseType')}</label>
          <select value={form.db_type} onChange={(e) => { const t = e.target.value; setForm({ ...form, db_type: t, port: DEFAULT_PORTS[t] ?? 3306 }); }}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 focus:outline-none focus:border-amber-500/50 transition-premium">
            <option value="mysql">{t('datasources.dbTypeMysql')}</option>
            <option value="postgresql">{t('datasources.dbTypePostgresql')}</option>
            <option value="oracle">{t('datasources.dbTypeOracle')}</option>
          </select>
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.host')}</label>
          <input value={form.host} onChange={(e) => setForm({ ...form, host: e.target.value })}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 font-mono focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.port')}</label>
          <input type="number" value={form.port} onChange={(e) => setForm({ ...form, port: parseInt(e.target.value) || 3306 })}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 font-mono focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.database')}</label>
          <input value={form.database_name} onChange={(e) => setForm({ ...form, database_name: e.target.value })} placeholder={t('datasources.databasePlaceholder')}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.username')}</label>
          <input value={form.username} onChange={(e) => setForm({ ...form, username: e.target.value })}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-gray-400">{t('datasources.password')}</label>
          <input type="password" value={form.password} onChange={(e) => setForm({ ...form, password: e.target.value })}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 focus:outline-none focus:border-amber-500/50 transition-premium" />
        </div>
      </div>
      <div className="flex items-center gap-3 mt-5">
        <button onClick={onCancel} className="text-xs text-gray-500 hover:text-gray-300 transition-premium">{t('common.cancel')}</button>
        <button onClick={onSubmit} disabled={submitting || !form.name || !form.database_name}
          className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 disabled:bg-obsidian-700 disabled:text-gray-600 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-premium active:translate-y-[1px]">
          {submitting ? t('datasources.adding') : t('datasources.connectDatabase')}
        </button>
      </div>
    </div>
  );
}

// ── Datasource access grants modal (admin only) ──
function GrantsModal({
  datasourceId,
  datasourceName,
  onClose,
}: {
  datasourceId: number;
  datasourceName: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [users, setUsers] = useState<User[]>([]);
  const [granted, setGranted] = useState<Set<number>>(new Set());
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([usersApi.list(), datasourcesApi.listGrants(datasourceId)])
      .then(([us, g]) => { setUsers(us); setGranted(new Set(g)); })
      .catch((e) => setErr(e instanceof Error ? e.message : 'Failed to load'))
      .finally(() => setLoading(false));
  }, [datasourceId]);

  const admins = users.filter((u) => u.role === 'admin');
  const members = users.filter((u) => u.role !== 'admin');

  const toggle = (id: number) =>
    setGranted((prev) => {
      const n = new Set(prev);
      if (n.has(id)) n.delete(id); else n.add(id);
      return n;
    });

  const save = async () => {
    setSaving(true);
    setErr(null);
    try {
      // Admins always have access implicitly, so only member grants are sent.
      const memberIds = new Set(members.map((m) => m.id));
      await datasourcesApi.setGrants(datasourceId, Array.from(granted).filter((id) => memberIds.has(id)));
      onClose();
    } catch (e) {
      setErr(e instanceof Error ? e.message : 'Failed to save');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="bg-obsidian-900 border border-obsidian-700 rounded-xl w-[420px] max-h-[80vh] flex flex-col shadow-2xl" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center justify-between px-5 py-3 border-b border-obsidian-700 flex-shrink-0">
          <h3 className="text-sm font-semibold text-gray-100 flex items-center gap-2">
            <Key size={15} className="text-amber-500" />
            {t('datasources.access.title', { name: datasourceName })}
          </h3>
          <button onClick={onClose} className="text-gray-500 hover:text-gray-300"><X size={16} /></button>
        </div>

        <div className="px-5 py-4 overflow-y-auto scrollbar-thin">
          <p className="text-[11px] text-gray-500 mb-3">{t('datasources.access.desc')}</p>

          {err && <div className="text-[11px] text-red-400 bg-red-500/10 rounded px-2 py-1.5 mb-3">{err}</div>}

          {loading ? (
            <div className="text-xs text-gray-500 py-4 text-center">{t('common.loading')}</div>
          ) : (
            <>
              {members.length === 0 && (
                <p className="text-[11px] text-gray-600 italic">{t('datasources.access.noMembers')}</p>
              )}
              <div className="space-y-1">
                {members.map((u) => (
                  <label key={u.id} className="flex items-center gap-2.5 px-2 py-1.5 rounded-lg hover:bg-obsidian-800/50 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={granted.has(u.id)}
                      onChange={() => toggle(u.id)}
                      className="accent-amber-500"
                    />
                    <div className="min-w-0 flex-1">
                      <div className="text-xs text-gray-200 truncate">{u.display_name || u.username}</div>
                      <div className="text-[10px] text-gray-500">@{u.username}</div>
                    </div>
                  </label>
                ))}
              </div>

              {admins.length > 0 && (
                <p className="text-[10px] text-gray-600 mt-3 pt-3 border-t border-obsidian-700/50">
                  {t('datasources.access.adminsNote', { names: admins.map((a) => a.username).join(', ') })}
                </p>
              )}
            </>
          )}
        </div>

        <div className="flex items-center justify-end gap-2 px-5 py-3 border-t border-obsidian-700 flex-shrink-0">
          <button onClick={onClose} className="text-[11px] text-gray-400 hover:text-gray-200 px-3 py-1.5 rounded-md border border-obsidian-700">
            {t('common.cancel')}
          </button>
          <button onClick={save} disabled={saving || loading} className="text-[11px] text-[#08080c] bg-amber-500 hover:bg-amber-400 font-semibold px-4 py-1.5 rounded-md disabled:opacity-50">
            {saving ? t('common.loading') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
