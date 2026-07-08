import { useEffect, useState, useRef, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  PaperPlaneRight,
  Sparkle,
  Database,
  User,
  Robot,
  Plus,
  Trash,
  CaretDown,
  CaretRight,
  Star,
  FloppyDisk,
  Folder,
  ThumbsUp,
  Stop,
  ChartBar,
  MagnifyingGlass,
  List,
  Lightbulb,
} from '@phosphor-icons/react';
import { useWebSocket, type WSMessage } from '../hooks/useWebSocket';
import { conversationsApi, datasourcesApi, queryApi, metricsApi, metricGroupsApi, type Conversation, type Message, type DataSource, type MetricGroup, type MetricPool, type MetricParam } from '../lib/api';
import { EmptyState, Select } from '../components/ui';

/** A query result pool shown in the conversation. `params` is present when the
 *  AI parameterized the query (uses {{name}} / [[ ]] placeholders). */
type ConvPool = { id: number; name: string; sql: string; rows: number; datasource_id: number; params?: MetricParam[] };

interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  pools?: ConvPool[];
}

// Map persisted DB messages into the UI shape. Assistant messages store the raw
// LLM JSON; we extract the human-readable explanation and reconstruct pools.
function mapDbMessages(msgs: Message[]): ChatMessage[] {
  return msgs.map((m) => {
    if (m.role === 'assistant') {
      try {
        const parsed = JSON.parse(m.content);
        if (parsed && typeof parsed.explanation === 'string') {
          return {
            id: String(m.id),
            role: 'assistant' as const,
            content: parsed.explanation,
            pools: m.metadata && typeof m.metadata === 'object' && 'pool_ids' in (m.metadata as Record<string, unknown>)
              ? ((m.metadata as Record<string, unknown>).pool_ids as number[]).map((pid, i) => ({
                  id: pid,
                  name: parsed.queries?.[i]?.label || `Query ${i + 1}`,
                  sql: parsed.queries?.[i]?.sql || '',
                  rows: 0,
                  datasource_id: parsed.queries?.[i]?.datasource_id || 1,
                  params: parsed.queries?.[i]?.params as MetricParam[] | undefined,
                }))
              : parsed.queries?.map((q: { sql: string; label?: string; datasource_id?: number; params?: MetricParam[] }, i: number) => ({
                  id: i,
                  name: q.label || `Query ${i + 1}`,
                  sql: q.sql || '',
                  rows: 0,
                  datasource_id: q.datasource_id || 1,
                  params: q.params,
                })) || undefined,
          };
        }
      } catch {
        // Not JSON — display as-is
      }
    }
    return {
      id: String(m.id),
      role: m.role as ChatMessage['role'],
      content: m.content,
    };
  });
}

export default function ConversationsPage() {
  const { t, i18n } = useTranslation();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [datasources, setDatasources] = useState<DataSource[]>([]);
  const [selectedDsId, setSelectedDsId] = useState<number | null>(() => {
    const saved = localStorage.getItem('conv-datasource-id');
    return saved ? parseInt(saved) : null;
  });
  const chatEndRef = useRef<HTMLDivElement>(null);
  const pendingPoolsRef = useRef<ChatMessage['pools']>([]);
  // When we just sent the first message of a brand-new conversation, the live
  // WS stream owns the UI — skip the destructive DB reload for that id once.
  const skipLoadRef = useRef<number | null>(null);

  // Load conversations and datasources
  const fetchConversations = useCallback(async () => {
    try {
      const data = await conversationsApi.list();
      setConversations(data);
    } catch {
      /* silent */
    }
  }, []);

  const [metrics, setMetrics] = useState<MetricPool[]>([]);
  const [showMetricsPicker, setShowMetricsPicker] = useState(false);
  const [metricSearch, setMetricSearch] = useState('');
  // Mobile: the conversation list is an overlay drawer toggled by a button.
  const [showList, setShowList] = useState(false);

  useEffect(() => {
    fetchConversations();
    datasourcesApi.list().then((ds) => {
      setDatasources(ds);
      if (ds.length > 0 && !selectedDsId) {
        // Prefer the locked default datasource, falling back to the first one.
        const target = ds.find((d) => d.is_default) ?? ds[0];
        setSelectedDsId(target.id);
        localStorage.setItem('conv-datasource-id', String(target.id));
      }
    }).catch(() => {});
    metricsApi.list().then(setMetrics).catch(() => {});
  }, [fetchConversations]);

  // Keep the metrics list fresh so already-favorited pools show as saved
  // (e.g. after saving here, or from the Metrics page).
  useEffect(() => {
    const refresh = () => { metricsApi.list().then(setMetrics).catch(() => {}); };
    window.addEventListener('metrics-updated', refresh);
    return () => window.removeEventListener('metrics-updated', refresh);
  }, []);

  // Load messages when active conversation changes, and resume any in-progress
  // async generation (poll until the server finishes, then show the result).
  useEffect(() => {
    if (!activeId) {
      setMessages([]);
      setStreaming(false);
      return;
    }

    // Brand-new conversation we just sent to: the live WS stream populates it.
    if (skipLoadRef.current === activeId) {
      skipLoadRef.current = null;
      return;
    }

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Only reload after polling when we actually observed an in-progress
    // generation finish. Without this, an already-idle conversation would be
    // fetched twice (initial reload + a redundant post-poll reload).
    let sawGenerating = false;

    const reload = async () => {
      try {
        const msgs = await conversationsApi.getMessages(activeId);
        if (cancelled) return;
        const mapped = mapDbMessages(msgs);
        // mapDbMessages defaults every pool's row count to 0 (the DB message
        // metadata only carries pool ids). Fetch the real row_count FIRST and
        // render once with correct values — rendering the 0 placeholder first
        // and patching afterwards causes a visible "0 -> N" flicker on reload.
        const poolIds = Array.from(
          new Set(mapped.flatMap((m) => m.pools?.map((p) => p.id) ?? []).filter((id) => id > 0)),
        );
        if (poolIds.length > 0) {
          const counts = await Promise.all(
            poolIds.map(async (id) => {
              try {
                const p = await queryApi.getPool(id);
                return [id, p.row_count ?? 0] as const;
              } catch {
                return [id, 0] as const;
              }
            }),
          );
          if (cancelled) return;
          const countMap = new Map(counts);
          for (const m of mapped) {
            if (m.pools) {
              m.pools = m.pools.map((p) => (countMap.has(p.id) ? { ...p, rows: countMap.get(p.id)! } : p));
            }
          }
        }
        if (!cancelled) setMessages(mapped);
      } catch {
        /* silent */
      }
    };

    const poll = async () => {
      if (cancelled) return;
      try {
        const s = await conversationsApi.getStatus(activeId);
        if (cancelled) return;
        if (s.generation_status === 'generating') {
          setStreaming(true);
          sawGenerating = true;
          timer = setTimeout(poll, 2000);
        } else {
          setStreaming(false);
          if (sawGenerating) {
            await reload();
            fetchConversations();
          }
          if (s.generation_status === 'failed' && s.generation_error && !cancelled) {
            setMessages((prev) => [
              ...prev,
              { id: `gen-error-${activeId}`, role: 'system', content: s.generation_error! },
            ]);
          }
        }
      } catch {
        /* silent */
      }
    };

    (async () => {
      await reload();
      if (!cancelled) poll();
    })();

    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [activeId]);

  const handleWSMessage = useCallback((msg: WSMessage) => {
    switch (msg.type) {
      case 'connected':
        break;

      case 'reasoning':
      case 'content':
        // LLM outputs raw JSON (not human-readable) — don't display it.
        // Just show the streaming/loading indicator.
        setStreaming(true);
        break;

      case 'explanation':
        if (msg.content) {
          setMessages((prev) => [
            ...prev,
            {
              id: `explain-${Date.now()}`,
              role: 'assistant',
              content: msg.content!,
            },
          ]);
        }
        break;

      case 'query_result': {
        const poolId = msg.pool_id as number;
        if (poolId) {
          pendingPoolsRef.current = [
            ...(pendingPoolsRef.current || []),
            {
              id: poolId,
              name: (msg.label as string) || `Query ${poolId}`,
              sql: (msg.sql as string) || '',
              rows: (msg.row_count as number) || 0,
              datasource_id: (msg.datasource_id as number) || 1,
              params: (msg.params as MetricParam[]) || undefined,
            },
          ];
        }
        break;
      }

      case 'query_error':
        setMessages((prev) => [
          ...prev,
          {
            id: `err-${Date.now()}`,
            role: 'system',
            content: `SQL Error: ${msg.message}`,
          },
        ]);
        break;

      case 'done': {
        setStreaming(false);
        const pools = pendingPoolsRef.current;
        pendingPoolsRef.current = [];

        setMessages((prev) => {
          // If we already have an explanation message, attach pools to it
          if (
            prev.length > 0 &&
            prev[prev.length - 1].role === 'assistant'
          ) {
            const last = prev[prev.length - 1];
            return [
              ...prev.slice(0, -1),
              { ...last, pools: pools && pools.length > 0 ? pools : last.pools },
            ];
          }
          // No explanation was sent — create a message from the done event
          return [
            ...prev,
            {
              id: `done-${Date.now()}`,
              role: 'assistant',
              content: msg.message || t('conv.analysisComplete'),
              pools: pools && pools.length > 0 ? pools : undefined,
            },
          ];
        });
        break;
      }

      case 'error':
        setStreaming(false);
        pendingPoolsRef.current = [];
        setMessages((prev) => [
          ...prev,
          {
            id: `err-${Date.now()}`,
            role: 'system',
            content: msg.message || 'Error',
          },
        ]);
        break;

      case 'interrupted':
        setStreaming(false);
        pendingPoolsRef.current = [];
        break;
    }
  }, [t]);

  const { send, isOpen } = useWebSocket({
    onMessage: handleWSMessage,
    onOpen: () => {},
    onClose: () => setStreaming(false),
    onError: () => setStreaming(false),
  });

  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  const handleSend = async () => {
    const text = input.trim();
    if (!text) return;

    // Create conversation if none active
    let convId = activeId;
    if (!convId) {
      try {
        const conv = await conversationsApi.create();
        convId = conv.id;
        skipLoadRef.current = convId; // live stream owns this fresh conversation
        setActiveId(convId);
        setConversations((prev) => [conv, ...prev]);
      } catch {
        return;
      }
    }

    setMessages((prev) => [
      ...prev,
      { id: `u-${Date.now()}`, role: 'user', content: text },
    ]);
    setInput('');
    pendingPoolsRef.current = [];

    send({ action: 'chat', query: text, conversation_id: convId, datasource_id: selectedDsId, lang: i18n.language });
  };

  const handleStop = () => {
    send({ action: 'stop' });
    setStreaming(false);
    pendingPoolsRef.current = [];
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleNewConv = async () => {
    try {
      const conv = await conversationsApi.create();
      setConversations((prev) => [conv, ...prev]);
      setActiveId(conv.id);
      setMessages([]);
    } catch {
      /* silent */
    }
  };

  const handleDeleteConv = async (id: number) => {
    try {
      await conversationsApi.delete(id);
      if (activeId === id) {
        setActiveId(null);
        setMessages([]);
      }
      setConversations((prev) => prev.filter((c) => c.id !== id));
    } catch {
      /* silent */
    }
  };

  const suggestionPrompts = [
    t('conv.suggestions.revenue'),
    t('conv.suggestions.retention'),
    t('conv.suggestions.compare'),
  ];

  const handlePickMetric = (m: MetricPool) => {
    setInput((prev) => {
      const ref = `分析指标「${m.name}」的数据情况`;
      return prev.trim() ? `${prev.trim()}\n${ref}` : ref;
    });
    if (m.datasource_id && m.datasource_id !== selectedDsId) {
      setSelectedDsId(m.datasource_id);
      localStorage.setItem('conv-datasource-id', String(m.datasource_id));
    }
    setShowMetricsPicker(false);
    setMetricSearch('');
  };

  // Group metrics by datasource, filtered by the search term.
  const metricGroups = (() => {
    const q = metricSearch.trim().toLowerCase();
    const filtered = q
      ? metrics.filter(
          (m) => m.name.toLowerCase().includes(q) || (m.description || '').toLowerCase().includes(q)
        )
      : metrics;
    const byDs = new Map<number, MetricPool[]>();
    for (const m of filtered) {
      const arr = byDs.get(m.datasource_id) || [];
      arr.push(m);
      byDs.set(m.datasource_id, arr);
    }
    return Array.from(byDs.entries()).map(([dsId, items]) => ({
      dsId,
      dsName: datasources.find((d) => d.id === dsId)?.name || `数据源 ${dsId}`,
      items,
    }));
  })();
  const totalFilteredMetrics = metricGroups.reduce((n, g) => n + g.items.length, 0);

  return (
    <div className="h-full flex relative">
      {/* Mobile overlay when the conversation list drawer is open */}
      {showList && (
        <div
          className="md:hidden absolute inset-0 z-30 bg-black/60"
          onClick={() => setShowList(false)}
        />
      )}
      {/* Conversation List Sidebar */}
      <aside
        className={`
          absolute md:static inset-y-0 left-0 z-40 w-64 md:w-56
          border-r border-obsidian-700 flex flex-col flex-shrink-0 bg-obsidian-950
          transform transition-transform duration-200 md:transform-none
          ${showList ? 'translate-x-0' : '-translate-x-full md:translate-x-0'}
        `}
      >
        <div className="px-3 py-3 border-b border-obsidian-700">
          <button
            onClick={handleNewConv}
            className="w-full flex items-center justify-center gap-1.5 bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs py-2 rounded-lg transition-premium active:translate-y-[1px]"
          >
            <Plus size={14} weight="bold" /> {t('conv.newChat')}
          </button>
        </div>
        <div className="flex-1 overflow-y-auto scrollbar-thin px-2 py-2 space-y-0.5">
          {conversations.length === 0 && (
            <p className="text-[10px] text-gray-600 text-center py-4 italic">
              {t('conv.noConversations')}
            </p>
          )}
          {conversations.map((c) => (
            <div
              key={c.id}
              className={`group flex items-center gap-1 rounded-lg transition-premium ${
                activeId === c.id
                  ? 'bg-amber-500/10 border-l-2 border-amber-500'
                  : 'hover:bg-obsidian-800'
              }`}
            >
              <button
                onClick={() => { setActiveId(c.id); setShowList(false); }}
                className={`flex-1 text-left px-2.5 py-2 text-xs truncate flex items-center gap-1.5 ${
                  activeId === c.id ? 'text-amber-500' : 'text-gray-400 hover:text-gray-200'
                }`}
              >
                {c.generation_status === 'generating' && (
                  <span className="w-1.5 h-1.5 rounded-full bg-amber-500 dot-breathe flex-shrink-0" title={t('conv.analyzing')} />
                )}
                <span className="truncate flex-1">{c.title || t('conv.titleWithId', { id: c.id })}</span>
                {c.created_at && (
                  <span className="text-[9px] text-gray-600 flex-shrink-0">
                    {new Date(c.created_at).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}
                  </span>
                )}
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  handleDeleteConv(c.id);
                }}
                className="opacity-0 group-hover:opacity-100 text-gray-600 hover:text-red-400 p-1 mr-1 transition-premium"
              >
                <Trash size={12} />
              </button>
            </div>
          ))}
        </div>
        <div className="px-3 py-2 border-t border-obsidian-700">
          <div className="flex items-center gap-1.5">
            <div
              className={`w-1.5 h-1.5 rounded-full ${
                isOpen ? 'bg-data-green dot-breathe' : 'bg-data-amber'
              }`}
            />
            <span className="text-[10px] text-gray-600">
              {isOpen ? t('conv.connected') : t('conv.reconnecting')}
            </span>
          </div>
        </div>
      </aside>

      {/* Chat Area */}
      <div className="flex-1 flex flex-col min-w-0">
        <div className="flex items-center justify-between px-3 md:px-6 py-3 border-b border-obsidian-700 gap-2">
          <div className="flex items-center gap-2 min-w-0">
            <button
              onClick={() => setShowList(true)}
              className="md:hidden w-8 h-8 flex items-center justify-center rounded-lg text-gray-400 hover:text-gray-200 hover:bg-obsidian-800 flex-shrink-0"
              aria-label={t('conv.newChat')}
            >
              <List size={18} />
            </button>
            <div className="min-w-0">
              <h1 className="text-sm font-bold text-gray-100 tracking-tight truncate">
                {activeId
                  ? t('conv.titleWithId', { id: activeId })
                  : t('conv.title')}
              </h1>
              <p className="text-[10px] text-gray-500 mt-0.5 hidden sm:block">
                {t('conv.description')}
              </p>
            </div>
          </div>
          {/* Workflow hint: save metrics first, then build reports */}
          <div className="hidden md:flex flex-1 items-center justify-center gap-1.5 min-w-0 px-2">
            <Lightbulb size={13} weight="fill" className="text-amber-400 flex-shrink-0 animate-pulse" />
            <span className="text-[11px] text-gray-400 truncate">{t('conv.hint.short')}</span>
          </div>
          {/* Datasource selector */}
          {datasources.length > 0 && (
            <div className="flex items-center gap-2 flex-shrink-0">
              <Database size={14} className="text-gray-500 hidden sm:block" />
              <Select
                value={selectedDsId ?? ''}
                onChange={(v) => { const id = parseInt(v) || null; setSelectedDsId(id); if (id) localStorage.setItem('conv-datasource-id', String(id)); else localStorage.removeItem('conv-datasource-id'); }}
                size="sm"
                className="max-w-[40vw] sm:max-w-none"
              >
                {datasources.map((ds) => (
                  <option key={ds.id} value={ds.id}>
                    {ds.name} ({ds.db_type})
                  </option>
                ))}
              </Select>
            </div>
          )}
        </div>

        <div className="flex-1 overflow-y-auto scrollbar-thin px-3 md:px-6 py-4 space-y-4">
          {messages.length === 0 && (
            <EmptyState
              icon={Sparkle}
              title={t('conv.empty.title')}
              description={t('conv.empty.description')}
              action={
                <div className="flex flex-wrap gap-2 justify-center">
                  {suggestionPrompts.map((prompt) => (
                    <button
                      key={prompt}
                      onClick={() => setInput(prompt)}
                      className="bg-obsidian-800 hover:bg-obsidian-700 border border-obsidian-700 text-gray-400 hover:text-gray-200 text-[11px] px-3 py-1.5 rounded-full transition-premium"
                    >
                      {prompt}
                    </button>
                  ))}
                </div>
              }
            />
          )}

          {messages.map((msg) => (
            <div
              key={msg.id}
              className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
            >
              <div
                className={`flex gap-2 max-w-[88%] md:max-w-[75%] group ${
                  msg.role === 'user' ? 'flex-row-reverse' : ''
                }`}
              >
                <div
                  className={`w-7 h-7 rounded-lg flex items-center justify-center flex-shrink-0 ${
                    msg.role === 'user'
                      ? 'bg-amber-500'
                      : msg.role === 'system'
                        ? 'bg-red-500/20'
                        : 'bg-obsidian-700'
                  }`}
                >
                  {msg.role === 'user' ? (
                    <User size={14} className="text-[#08080c]" />
                  ) : msg.role === 'system' ? (
                    <Sparkle size={14} className="text-red-400" />
                  ) : (
                    <Robot size={14} className="text-amber-500" />
                  )}
                </div>
                <div
                  className={`px-3 py-2 rounded-xl text-xs leading-relaxed ${
                    msg.role === 'user'
                      ? 'bg-amber-500/10 border border-amber-500/20 rounded-br-sm text-gray-200'
                      : msg.role === 'system'
                        ? 'bg-red-500/10 border border-red-500/20 text-red-400'
                        : 'bg-obsidian-800 border border-obsidian-700 text-gray-300'
                  }`}
                >
                  <p className="whitespace-pre-wrap">{msg.content}</p>
                  {/* Like button for assistant messages — saves as training example */}
                  {msg.role === 'assistant' && (
                    <LikeButton
                      messages={messages}
                      msgId={msg.id}
                      content={msg.content}
                      datasourceId={selectedDsId}
                      t={t}
                    />
                  )}
                  {msg.pools && msg.pools.length > 0 && (
                    <div className="mt-2 space-y-1.5 pt-2 border-t border-obsidian-700/50">
                      <div className="flex items-center justify-between">
                        <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium">
                          {t('conv.queryResults')}
                        </span>
                        {msg.pools.length > 0 && (
                          <BatchSaveButton pools={msg.pools} metrics={metrics} t={t} />
                        )}
                      </div>
                      {msg.pools.map((p) => (
                        <PoolResultCard key={p.id} pool={p} metrics={metrics} t={t} />
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          ))}

          {streaming && (
            <div className="flex justify-start">
              <div className="flex gap-2">
                <div className="w-7 h-7 rounded-lg bg-obsidian-700 flex items-center justify-center flex-shrink-0">
                  <Robot size={14} className="text-amber-500" />
                </div>
                <div className="bg-obsidian-800 border border-obsidian-700 rounded-xl px-3 py-2">
                  <div className="flex items-center gap-1.5">
                    <div
                      className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-bounce"
                      style={{ animationDelay: '0ms' }}
                    />
                    <div
                      className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-bounce"
                      style={{ animationDelay: '150ms' }}
                    />
                    <div
                      className="w-1.5 h-1.5 rounded-full bg-amber-500 animate-bounce"
                      style={{ animationDelay: '300ms' }}
                    />
                    <span className="text-[10px] text-gray-600 ml-1">
                      {t('conv.analyzing')}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          )}
          <div ref={chatEndRef} />
        </div>

        {/* Input composer */}
        <div className="border-t border-obsidian-700 px-3 md:px-6 py-3">
          <div className="relative">
            {/* Metrics popover */}
            {showMetricsPicker && (
              <>
                <div className="fixed inset-0 z-30" onClick={() => setShowMetricsPicker(false)} />
                <div className="absolute bottom-full mb-2 left-0 right-0 z-40 bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl overflow-hidden">
                  <div className="px-3 py-2 border-b border-obsidian-700 flex items-center gap-2">
                    <ChartBar size={13} className="text-amber-500" />
                    <span className="text-[11px] text-gray-300 font-medium">从指标库选择</span>
                    <span className="text-[10px] text-gray-600 ml-auto">{totalFilteredMetrics} 个指标</span>
                  </div>
                  {metrics.length > 0 && (
                    <div className="px-2 py-2 border-b border-obsidian-700">
                      <div className="relative">
                        <MagnifyingGlass size={12} className="absolute left-2 top-1/2 -translate-y-1/2 text-gray-600" />
                        <input
                          autoFocus
                          value={metricSearch}
                          onChange={(e) => setMetricSearch(e.target.value)}
                          placeholder="搜索指标名称或描述…"
                          className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg pl-7 pr-2 py-1.5 text-[11px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50"
                        />
                      </div>
                    </div>
                  )}
                  <div className="max-h-64 overflow-y-auto scrollbar-thin py-1">
                    {metrics.length === 0 ? (
                      <p className="text-[10px] text-gray-600 text-center py-5 italic">暂无指标，请先在指标库中创建</p>
                    ) : totalFilteredMetrics === 0 ? (
                      <p className="text-[10px] text-gray-600 text-center py-5 italic">没有匹配的指标</p>
                    ) : (
                      metricGroups.map((grp) => (
                        <div key={grp.dsId} className="mb-1">
                          <div className="px-3 py-1 flex items-center gap-1.5 sticky top-0 bg-obsidian-900">
                            <Database size={10} className="text-gray-600" />
                            <span className="text-[9px] text-gray-500 uppercase tracking-wide font-medium">{grp.dsName}</span>
                          </div>
                          {grp.items.map((m) => (
                            <button
                              key={m.id}
                              onClick={() => handlePickMetric(m)}
                              className="w-full flex items-start gap-2 px-3 py-1.5 text-left hover:bg-obsidian-800 transition-premium group"
                            >
                              <ChartBar size={12} className="text-amber-500/60 flex-shrink-0 mt-0.5" />
                              <span className="min-w-0 flex-1">
                                <span className="block text-[11px] text-gray-300 group-hover:text-gray-100 truncate">{m.name}</span>
                                {m.description && (
                                  <span className="block text-[9px] text-gray-600 truncate">{m.description}</span>
                                )}
                              </span>
                            </button>
                          ))}
                        </div>
                      ))
                    )}
                  </div>
                </div>
              </>
            )}

            {/* Composer */}
            <div className="bg-obsidian-800 border border-obsidian-700 rounded-xl focus-within:border-amber-500/50 transition-premium">
              <div className="flex items-center gap-2 px-2.5 pt-2">
                <button
                  type="button"
                  onClick={() => setShowMetricsPicker((v) => !v)}
                  className={`flex items-center gap-1 text-[10px] px-2 py-1 rounded-md border transition-premium ${
                    showMetricsPicker
                      ? 'text-amber-500 border-amber-500/40 bg-amber-500/10'
                      : 'text-gray-400 border-obsidian-700 hover:text-amber-500 hover:border-amber-500/30'
                  }`}
                >
                  <ChartBar size={12} weight="fill" /> 指标库
                  <CaretDown size={9} className={`transition-transform ${showMetricsPicker ? 'rotate-180' : ''}`} />
                </button>
              </div>
              <div className="relative">
                <textarea
                  value={input}
                  onChange={(e) => setInput(e.target.value)}
                  onKeyDown={handleKeyDown}
                  placeholder={t('conv.placeholder')}
                  disabled={streaming}
                  rows={1}
                  className="w-full bg-transparent border-0 pl-3 pr-11 py-2 text-xs text-gray-200 placeholder-gray-600 focus:outline-none disabled:opacity-50 resize-none"
                  style={{ minHeight: '38px', maxHeight: '120px' }}
                  onInput={(e) => {
                    const target = e.target as HTMLTextAreaElement;
                    target.style.height = 'auto';
                    target.style.height = Math.min(target.scrollHeight, 120) + 'px';
                  }}
                />
                <button
                  onClick={streaming ? handleStop : handleSend}
                  disabled={streaming ? false : (!input.trim() || !isOpen)}
                  title={streaming ? t('conv.stop') : undefined}
                  className={`absolute right-2.5 bottom-2 transition-premium ${
                    streaming
                      ? 'text-red-400 hover:text-red-300'
                      : 'text-gray-500 hover:text-amber-500 disabled:text-gray-700'
                  }`}
                >
                  {streaming ? <Stop size={16} weight="fill" /> : <PaperPlaneRight size={16} weight="fill" />}
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Expandable Pool Result Card ──
/** A pool is already favorited if a metric with the same datasource + SQL
 *  exists. Data pool ids change per query, so we match on SQL, not id. */
function poolAlreadySaved(
  pool: { sql: string; datasource_id: number },
  metrics: MetricPool[],
): boolean {
  const sql = pool.sql.trim();
  return metrics.some((m) => m.datasource_id === pool.datasource_id && m.sql_query.trim() === sql);
}

function PoolResultCard({
  pool,
  metrics,
  t,
}: {
  pool: ConvPool;
  metrics: MetricPool[];
  t: (k: string, opts?: Record<string, unknown>) => string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [data, setData] = useState<{ columns: string[]; rows: Record<string, unknown>[] } | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  // Reflect whether this pool is already in the metric library (persists across
  // conversation reloads, unlike a purely local "saved" flag).
  const alreadySaved = poolAlreadySaved(pool, metrics);
  const [savedLocal, setSavedLocal] = useState(false);
  const saved = savedLocal || alreadySaved;

  const handleExpand = async () => {
    if (expanded) {
      setExpanded(false);
      return;
    }
    setExpanded(true);
    if (!data) {
      try {
        setLoading(true);
        const poolData = await queryApi.getPool(pool.id);
        if (poolData.result_cache) {
          const rows = Array.isArray(poolData.result_cache) ? poolData.result_cache as Record<string, unknown>[] : [];
          const columns = rows.length > 0 ? Object.keys(rows[0]) : [];
          setData({ columns, rows });
        }
      } catch {
        // silent
      } finally {
        setLoading(false);
      }
    }
  };

  const handleSaveToMetrics = async () => {
    if (saved) return;
    try {
      setSaving(true);
      await metricsApi.create({
        name: pool.name,
        sql_query: pool.sql,
        datasource_id: pool.datasource_id,
        source_pool_id: pool.id,
      });
      setSavedLocal(true);
      window.dispatchEvent(new Event('metrics-updated'));
    } catch {
      // silent
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="bg-obsidian-900/50 rounded-md overflow-hidden">
      {/* Header — clickable to expand */}
      <div
        onClick={handleExpand}
        className="flex items-center gap-2 px-2 py-1.5 cursor-pointer hover:bg-obsidian-800/50 transition-premium"
      >
        {expanded ? (
          <CaretDown size={10} className="text-gray-500 flex-shrink-0" />
        ) : (
          <CaretRight size={10} className="text-gray-500 flex-shrink-0" />
        )}
        <Database size={12} className="text-data-green flex-shrink-0" />
        <span className="font-mono text-[10px] text-data-green truncate flex-1">
          {pool.name}
        </span>
        {pool.params && pool.params.length > 0 && (
          <span
            className="text-[8px] text-amber-500 bg-amber-500/10 border border-amber-500/20 rounded px-1 py-0.5 flex-shrink-0"
            title={pool.params.map((p) => p.label || p.name).join(', ')}
          >
            {t('conv.paramBadge', { count: pool.params.length })}
          </span>
        )}
        <span className="text-[9px] text-gray-500 flex-shrink-0">
          {pool.rows.toLocaleString()} rows
        </span>
      </div>

      {/* Expanded: show SQL + data table + save button */}
      {expanded && (
        <div className="border-t border-obsidian-700/50">
          {/* SQL */}
          <div className="px-2 py-1.5 bg-obsidian-950/50">
            <code className="text-[9px] text-gray-500 font-mono block whitespace-pre-wrap break-all">
              {pool.sql}
            </code>
          </div>

          {/* Data table */}
          {loading && (
            <div className="px-2 py-3 flex items-center gap-1.5">
              <div className="w-2.5 h-2.5 border border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
              <span className="text-[9px] text-gray-600">{t('common.loading')}</span>
            </div>
          )}

          {data && data.rows.length > 0 && (
            <div className="overflow-x-auto max-h-[200px] overflow-y-auto scrollbar-thin">
              <table className="w-full text-[9px]">
                <thead className="sticky top-0 bg-obsidian-900">
                  <tr className="border-b border-obsidian-700">
                    {data.columns.map((col) => (
                      <th
                        key={col}
                        className="px-2 py-1 text-left text-gray-500 font-medium font-mono whitespace-nowrap"
                      >
                        {col}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {data.rows.slice(0, 20).map((row, ri) => (
                    <tr key={ri} className="border-b border-obsidian-700/30 hover:bg-obsidian-800/30">
                      {data.columns.map((col) => (
                        <td key={col} className="px-2 py-1 text-gray-300 font-mono whitespace-nowrap max-w-[150px] truncate">
                          {String(row[col] ?? '')}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
              {data.rows.length > 20 && (
                <div className="px-2 py-1 text-[8px] text-gray-600 border-t border-obsidian-700/30">
                  {t('conv.showingRows', { shown: 20, total: data.rows.length })}
                </div>
              )}
            </div>
          )}

          {data && data.rows.length === 0 && (
            <div className="px-2 py-2 text-[9px] text-gray-600 italic">
              {t('conv.noRows')}
            </div>
          )}

          {/* Actions */}
          <div className="flex items-center gap-2 px-2 py-1.5 border-t border-obsidian-700/50">
            <button
              onClick={handleSaveToMetrics}
              disabled={saving || saved}
              className={`flex items-center gap-1 text-[9px] px-2 py-1 rounded transition-premium ${
                saved
                  ? 'text-data-green bg-data-green/10'
                  : 'text-amber-500 hover:text-amber-400 hover:bg-amber-500/10'
              }`}
            >
              <Star size={10} weight={saved ? 'fill' : 'regular'} />
              {saved ? t('conv.savedToMetrics') : t('conv.saveToMetrics')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Batch Save All Pools to Metrics ──
function BatchSaveButton({
  pools,
  metrics,
  t,
}: {
  pools: ConvPool[];
  metrics: MetricPool[];
  t: (k: string, opts?: Record<string, unknown>) => string;
}) {
  const [saving, setSaving] = useState(false);
  // Consider the batch saved once every pool already exists in the library.
  const allAlreadySaved = pools.length > 0 && pools.every((p) => poolAlreadySaved(p, metrics));
  const [savedLocal, setSavedLocal] = useState(false);
  const saved = savedLocal || allAlreadySaved;
  const [showPicker, setShowPicker] = useState(false);
  const [groups, setGroups] = useState<MetricGroup[]>([]);
  const [newGroupName, setNewGroupName] = useState('');
  const [creatingGroup, setCreatingGroup] = useState(false);

  const loadGroups = async () => {
    try {
      const g = await metricGroupsApi.list();
      setGroups(g);
    } catch { /* silent */ }
  };

  const handleOpen = () => {
    if (saved) return;
    setShowPicker(true);
    loadGroups();
  };

  const handleSaveToGroup = async (groupId: number | null) => {
    setSaving(true);
    try {
      await Promise.all(
        pools.map((p) =>
          metricsApi.create({
            name: p.name,
            sql_query: p.sql,
            datasource_id: p.datasource_id,
            source_pool_id: p.id,
            group_id: groupId,
          })
        )
      );
      setSavedLocal(true);
      setShowPicker(false);
      window.dispatchEvent(new Event('metrics-updated'));
    } catch {
      // silent
    } finally {
      setSaving(false);
    }
  };

  const handleCreateAndSave = async () => {
    if (!newGroupName.trim()) return;
    setCreatingGroup(true);
    try {
      const group = await metricGroupsApi.create({ name: newGroupName.trim() });
      await handleSaveToGroup(group.id);
      setNewGroupName('');
    } catch {
      // silent
    } finally {
      setCreatingGroup(false);
    }
  };

  return (
    <div className="relative">
      <button
        onClick={handleOpen}
        disabled={saving || saved}
        className={`flex items-center gap-1 text-[9px] px-2 py-0.5 rounded transition-premium ${
          saved
            ? 'text-data-green bg-data-green/10'
            : 'text-amber-500 hover:text-amber-400 hover:bg-amber-500/10'
        }`}
      >
        <FloppyDisk size={10} weight={saved ? 'fill' : 'regular'} />
        {saved
          ? t('conv.batchSaved', { count: pools.length })
          : saving
            ? t('common.loading')
            : t('conv.batchSaveAll', { count: pools.length })}
      </button>

      {/* Group picker dropdown */}
      {showPicker && (
        <div className="absolute right-0 top-6 z-50 bg-obsidian-900 border border-obsidian-700 rounded-lg shadow-2xl w-56 overflow-hidden">
          <div className="px-3 py-2 border-b border-obsidian-700">
            <span className="text-[10px] text-gray-400 font-medium">{t('conv.selectGroup')}</span>
          </div>
          <div className="max-h-40 overflow-y-auto scrollbar-thin">
            {/* Ungrouped option */}
            <button
              onClick={() => handleSaveToGroup(null)}
              disabled={saving}
              className="w-full flex items-center gap-2 px-3 py-2 text-left text-[10px] text-gray-300 hover:bg-obsidian-800 transition-premium disabled:opacity-50"
            >
              <Database size={11} className="text-gray-500" />
              {t('conv.ungrouped')}
            </button>
            {/* Existing groups */}
            {groups.map((g) => (
              <button
                key={g.id}
                onClick={() => handleSaveToGroup(g.id)}
                disabled={saving}
                className="w-full flex items-center gap-2 px-3 py-2 text-left text-[10px] text-gray-300 hover:bg-obsidian-800 transition-premium disabled:opacity-50"
              >
                <Folder size={11} className="text-amber-500/60" />
                {g.name}
              </button>
            ))}
          </div>
          {/* Create new group */}
          <div className="border-t border-obsidian-700 px-2 py-2">
            <div className="flex items-center gap-1.5">
              <input
                type="text"
                value={newGroupName}
                onChange={(e) => setNewGroupName(e.target.value)}
                onKeyDown={(e) => { if (e.key === 'Enter') handleCreateAndSave(); }}
                placeholder={t('conv.newGroupPlaceholder')}
                className="flex-1 bg-obsidian-800 border border-obsidian-700 rounded px-2 py-1 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium"
              />
              <button
                onClick={handleCreateAndSave}
                disabled={!newGroupName.trim() || creatingGroup}
                className="text-[9px] text-amber-500 hover:text-amber-400 font-medium px-1.5 py-1 rounded transition-premium disabled:opacity-50"
              >
                <Plus size={12} />
              </button>
            </div>
          </div>
          {/* Cancel */}
          <div className="border-t border-obsidian-700 px-3 py-1.5">
            <button
              onClick={() => setShowPicker(false)}
              className="text-[9px] text-gray-500 hover:text-gray-300 transition-premium"
            >
              {t('common.cancel')}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Like Button — saves Q&A as training example ──
function LikeButton({
  messages,
  msgId,
  content,
  datasourceId,
  t,
}: {
  messages: { id: string; role: string; content: string }[];
  msgId: string;
  content: string;
  datasourceId: number | null;
  t: (k: string) => string;
}) {
  const [liked, setLiked] = useState(false);

  const handleLike = async () => {
    if (liked || !datasourceId) return;
    // Find the user message before this assistant message
    const idx = messages.findIndex((m) => m.id === msgId);
    const userMsg = idx > 0 ? messages.slice(0, idx).reverse().find((m) => m.role === 'user') : null;
    if (!userMsg) return;

    try {
      await fetch('/api/ai-examples', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${localStorage.getItem('token') || ''}`,
        },
        body: JSON.stringify({
          datasource_id: datasourceId,
          question: userMsg.content,
          answer: content,
          category: 'sql',
        }),
      });
      setLiked(true);
    } catch { /* silent */ }
  };

  return (
    <button
      onClick={handleLike}
      disabled={liked || !datasourceId}
      className={`mt-1.5 flex items-center gap-1 text-[9px] transition-premium ${
        liked
          ? 'text-data-green cursor-default'
          : 'text-gray-600 hover:text-amber-500 opacity-0 group-hover:opacity-100'
      }`}
      title={liked ? t('conv.likedHint') : t('conv.likeHint')}
    >
      <ThumbsUp size={11} weight={liked ? 'fill' : 'regular'} />
      {liked ? t('conv.liked') : ''}
    </button>
  );
}
