import { useState, useRef, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import {
  PaperPlaneRight,
  User,
  Robot,
  Database,
  Sparkle,
  Plus,
  Star,
} from '@phosphor-icons/react';
import { useWebSocket, type WSMessage } from '../hooks/useWebSocket';
import { useDataPoolStore, type UIDataPool } from '../stores/dataPoolStore';
import { conversationsApi, datasourcesApi, metricsApi, type MetricPool, type DataSource } from '../lib/api';

interface ChatMsg {
  id: string;
  role: 'user' | 'assistant' | 'system';
  content: string;
  pools?: { id: number; name: string; rows: number }[];
}

export default function AIPanel() {
  const { t, i18n } = useTranslation();
  const [searchParams] = useSearchParams();
  const { pools, addPool } = useDataPoolStore();
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState('');
  const [streaming, setStreaming] = useState(false);
  const [convId, setConvId] = useState<number | null>(null);

  // Get active datasource from URL context (datasources page uses ?ds=)
  const activeDsId = searchParams.get('ds') ? parseInt(searchParams.get('ds')!) : null;
  // Get active metric from URL context (metrics page uses ?id=)
  const activeMetricId = searchParams.get('id') ? parseInt(searchParams.get('id')!) : null;
  const [activeMetric, setActiveMetric] = useState<MetricPool | null>(null);
  const [activeDs, setActiveDs] = useState<DataSource | null>(null);
  const [sqlFavorites, setSqlFavorites] = useState<{ name: string; sql: string }[]>([]);
  const chatEndRef = useRef<HTMLDivElement>(null);
  const pendingPoolsRef = useRef<ChatMsg['pools']>([]);

  // Load active datasource details
  useEffect(() => {
    if (activeDsId) {
      datasourcesApi.get(activeDsId).then(setActiveDs).catch(() => setActiveDs(null));
    } else {
      setActiveDs(null);
    }
  }, [activeDsId]);

  // Load SQL favorites from localStorage
  useEffect(() => {
    try {
      const raw = localStorage.getItem('sql-favorites');
      if (raw) {
        const all = JSON.parse(raw) as { name: string; sql: string; dsId: number | null }[];
        setSqlFavorites(activeDsId ? all.filter((s) => s.dsId === activeDsId || s.dsId === null) : all);
      }
    } catch { setSqlFavorites([]); }
  }, [activeDsId]);

  // Load active metric details
  useEffect(() => {
    if (activeMetricId) {
      metricsApi.get(activeMetricId).then(setActiveMetric).catch(() => setActiveMetric(null));
    } else {
      setActiveMetric(null);
    }
  }, [activeMetricId]);

  // Scroll to bottom on new messages
  useEffect(() => {
    chatEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages, streaming]);

  const handleWSMessage = useCallback(
    (msg: WSMessage) => {
      switch (msg.type) {
        case 'connected':
          break;

        case 'reasoning':
        case 'content':
          // LLM outputs raw JSON — don't display it.
          // Just show the streaming/loading indicator.
          setStreaming(true);
          break;

        case 'explanation':
          // This is the human-readable explanation extracted from the JSON.
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
            addPool({
              id: poolId,
              name: (msg.label as string) || `Query ${poolId}`,
              sql_query: (msg.sql as string) || '',
              datasource_id: (msg.datasource_id as number) || 1,
              row_count: (msg.row_count as number) || 0,
              selected: false,
            });
            pendingPoolsRef.current = [
              ...(pendingPoolsRef.current || []),
              {
                id: poolId,
                name: (msg.label as string) || `Query ${poolId}`,
                rows: (msg.row_count as number) || 0,
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
          const collectedPools = pendingPoolsRef.current;
          pendingPoolsRef.current = [];

          // Attach pool info to the last assistant message if any
          if (collectedPools && collectedPools.length > 0) {
            setMessages((prev) => {
              const lastIdx = prev.length - 1;
              if (lastIdx >= 0 && prev[lastIdx].role === 'assistant') {
                const updated = [...prev];
                updated[lastIdx] = { ...updated[lastIdx], pools: collectedPools };
                return updated;
              }
              // No assistant message yet — create one
              return [
                ...prev,
                {
                  id: `done-${Date.now()}`,
                  role: 'assistant',
                  content: msg.message || t('aiPanel.analysisComplete'),
                  pools: collectedPools,
                },
              ];
            });
          } else if (msg.message) {
            // No pools, but there's a message
            setMessages((prev) => {
              if (
                prev.length > 0 &&
                prev[prev.length - 1].role === 'assistant'
              ) {
                return prev; // explanation already shown
              }
              return [
                ...prev,
                {
                  id: `done-${Date.now()}`,
                  role: 'assistant',
                  content: msg.message!,
                },
              ];
            });
          }
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
      }
    },
    [addPool, t]
  );

  const { send, isOpen } = useWebSocket({
    onMessage: handleWSMessage,
    onOpen: () => {},
    onClose: () => setStreaming(false),
    onError: () => setStreaming(false),
  });

  const handleSend = async () => {
    const text = input.trim();
    if (!text) return;

    // Create conversation if none
    let cid = convId;
    if (!cid) {
      try {
        const conv = await conversationsApi.create();
        cid = conv.id;
        setConvId(cid);
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

    // Build context prefix for AI
    let fullQuery = text;
    const contextParts: string[] = [];

    if (activeDs) {
      contextParts.push(`[数据源: "${activeDs.name}" (${activeDs.db_type})]`);
    }
    if (activeMetric) {
      contextParts.push(`[当前指标: "${activeMetric.name}", SQL: ${activeMetric.sql_query}]`);
    }
    if (sqlFavorites.length > 0 && !activeMetric) {
      const favList = sqlFavorites.slice(0, 5).map((f) => `  - ${f.name}: ${f.sql}`).join('\n');
      contextParts.push(`[收藏的SQL:\n${favList}]`);
    }

    if (contextParts.length > 0) {
      fullQuery = contextParts.join('\n') + '\n\n' + text;
    }

    send({ action: 'chat', query: fullQuery, conversation_id: cid, datasource_id: activeDsId || (activeMetric?.datasource_id ?? null), lang: i18n.language });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const handleNewChat = async () => {
    setMessages([]);
    setConvId(null);
    pendingPoolsRef.current = [];
    setStreaming(false);
  };

  const formatRows = (n: number) => {
    if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
    return String(n);
  };

  return (
    <aside className="w-[260px] h-full bg-obsidian-900 border-l border-obsidian-700 flex flex-col flex-shrink-0">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2.5 border-b border-obsidian-700">
        <div className="flex items-center gap-2">
          <span className="text-amber-500 text-[11px] font-bold tracking-wide">
            AI ASSISTANT
          </span>
          <span
            className={`w-1.5 h-1.5 rounded-full ${isOpen ? 'bg-data-green dot-breathe' : 'bg-red-500'} flex-shrink-0`}
            title={isOpen ? 'Connected' : 'Connecting...'}
          />
        </div>
        <button
          onClick={handleNewChat}
          className="flex items-center gap-1 text-[10px] text-gray-500 hover:text-gray-300 transition-premium"
        >
          <Plus size={12} />
          {t('aiPanel.newChat')}
        </button>
      </div>

      {/* Active context indicator */}
      {(activeDs || activeMetric) && (
        <div className="px-3 py-1.5 border-b border-obsidian-700 bg-amber-500/5">
          {activeDs && (
            <div className="flex items-center gap-1.5">
              <Database size={10} className="text-data-green flex-shrink-0" />
              <span className="text-[9px] text-data-green font-medium truncate">{activeDs.name}</span>
              <span className="text-[8px] text-gray-600">({activeDs.db_type})</span>
              {sqlFavorites.length > 0 && (
                <span className="text-[8px] text-gray-600 ml-auto">{sqlFavorites.length} SQL</span>
              )}
            </div>
          )}
          {activeMetric && (
            <div className="flex items-center gap-1.5 mt-0.5">
              <Star size={10} className="text-amber-500 flex-shrink-0" weight="fill" />
              <span className="text-[9px] text-amber-500/80 font-medium truncate">{activeMetric.name}</span>
            </div>
          )}
        </div>
      )}

      {/* Chat Messages */}
      <div className="flex-1 overflow-y-auto scrollbar-thin px-2 py-2 space-y-2">
        {messages.length === 0 && pools.length === 0 && !streaming && (
          <div className="text-center py-8">
            <Sparkle size={24} className="text-amber-500/30 mx-auto mb-2" />
            <p className="text-[10px] text-gray-500 leading-relaxed px-2">
              {t('aiPanel.askData')}
            </p>
          </div>
        )}

        {/* Chat messages */}
        {messages.map((msg) => (
          <div
            key={msg.id}
            className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
          >
            <div
              className={`flex gap-1.5 max-w-[90%] ${msg.role === 'user' ? 'flex-row-reverse' : ''}`}
            >
              <div
                className={`w-5 h-5 rounded flex items-center justify-center flex-shrink-0 mt-0.5 ${
                  msg.role === 'user' ? 'bg-amber-500' : 'bg-obsidian-700'
                }`}
              >
                {msg.role === 'user' ? (
                  <User size={10} className="text-[#08080c]" />
                ) : msg.role === 'system' ? (
                  <Sparkle size={10} className="text-red-400" />
                ) : (
                  <Robot size={10} className="text-amber-500" />
                )}
              </div>
              <div
                className={`px-2 py-1.5 rounded-lg text-[10px] leading-relaxed ${
                  msg.role === 'user'
                    ? 'bg-amber-500/10 border border-amber-500/20 text-gray-200'
                    : msg.role === 'system'
                      ? 'bg-red-500/10 border border-red-500/20 text-red-400'
                      : 'bg-obsidian-800 border border-obsidian-700 text-gray-300'
                }`}
              >
                <p className="whitespace-pre-wrap break-words">{msg.content}</p>
                {/* Inline pool results */}
                {msg.pools && msg.pools.length > 0 && (
                  <div className="mt-1.5 pt-1.5 border-t border-obsidian-700/50 space-y-1">
                    {msg.pools.map((p) => (
                      <div
                        key={p.id}
                        className="flex items-center gap-1.5 text-[9px]"
                      >
                        <Database size={10} className="text-data-green flex-shrink-0" />
                        <span className="font-mono text-data-green truncate">
                          {p.name}
                        </span>
                        <span className="text-gray-600 flex-shrink-0">
                          {formatRows(p.rows)}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </div>
        ))}

        {/* Streaming indicator */}
        {streaming && (
          <div className="flex justify-start">
            <div className="flex gap-1.5">
              <div className="w-5 h-5 rounded bg-obsidian-700 flex items-center justify-center flex-shrink-0">
                <Robot size={10} className="text-amber-500" />
              </div>
              <div className="bg-obsidian-800 border border-obsidian-700 rounded-lg px-3 py-1.5">
                <div className="flex items-center gap-1">
                  <div
                    className="w-1 h-1 rounded-full bg-amber-500 animate-bounce"
                    style={{ animationDelay: '0ms' }}
                  />
                  <div
                    className="w-1 h-1 rounded-full bg-amber-500 animate-bounce"
                    style={{ animationDelay: '150ms' }}
                  />
                  <div
                    className="w-1 h-1 rounded-full bg-amber-500 animate-bounce"
                    style={{ animationDelay: '300ms' }}
                  />
                  <span className="text-[9px] text-gray-600 ml-1">
                    {t('aiPanel.thinking')}
                  </span>
                </div>
              </div>
            </div>
          </div>
        )}

        {/* Data pool results (read-only display) */}
        {pools.length > 0 && (
          <div className="border-t border-obsidian-700 pt-2 mt-1">
            <div className="text-[9px] text-gray-500 font-medium mb-1.5 px-1">
              {t('aiPanel.dataPools')}
            </div>
            <div className="space-y-1">
              {pools.map((pool) => (
                <PoolCard key={pool.id} pool={pool} formatRows={formatRows} t={t} />
              ))}
            </div>
          </div>
        )}

        <div ref={chatEndRef} />
      </div>

      {/* Input */}
      <div className="border-t border-obsidian-700 p-2">
        <div className="relative">
          <input
            type="text"
            placeholder={activeMetric ? t('aiPanel.metricPlaceholder') : t('aiPanel.placeholder')}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={streaming}
            className="w-full bg-obsidian-800 border border-obsidian-700 rounded-lg pl-2.5 pr-8 py-1.5 text-[10px] text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/50 transition-premium disabled:opacity-50"
          />
          <button
            onClick={handleSend}
            disabled={!input.trim() || streaming || !isOpen}
            className="absolute right-1.5 top-1/2 -translate-y-1/2 text-gray-500 hover:text-amber-500 disabled:text-gray-700 transition-premium"
          >
            <PaperPlaneRight size={13} weight="fill" />
          </button>
        </div>
      </div>
    </aside>
  );
}

// ── Pool Card with save-to-metrics ──
function PoolCard({
  pool,
  formatRows,
  t,
}: {
  pool: UIDataPool;
  formatRows: (n: number) => string;
  t: (k: string, opts?: Record<string, unknown>) => string;
}) {
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const handleSave = async () => {
    if (saving || saved) return;
    setSaving(true);
    try {
      await metricsApi.create({
        name: pool.name,
        sql_query: pool.sql_query,
        datasource_id: pool.datasource_id,
        source_pool_id: pool.id,
      });
      setSaved(true);
      window.dispatchEvent(new Event('metrics-updated'));
    } catch {
      // silent
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="bg-obsidian-800 border border-obsidian-700 rounded-md p-1.5 group">
      <div className="flex items-start gap-1.5">
        <Database size={11} className="text-data-green flex-shrink-0 mt-0.5" />
        <div className="min-w-0 flex-1">
          <div className="text-[9px] text-data-green font-semibold font-mono truncate">
            {pool.name}
          </div>
          <div className="text-[8px] text-gray-500 mt-0.5">
            {formatRows(pool.row_count)} rows
          </div>
        </div>
        <button
          onClick={handleSave}
          disabled={saving || saved}
          title={saved ? t('aiPanel.savedToMetrics') : t('aiPanel.saveToMetrics')}
          className={`flex-shrink-0 transition-premium ${
            saved
              ? 'text-amber-500'
              : 'text-gray-600 hover:text-amber-500 opacity-0 group-hover:opacity-100'
          }`}
        >
          <Star size={12} weight={saved ? 'fill' : 'regular'} />
        </button>
      </div>
    </div>
  );
}
