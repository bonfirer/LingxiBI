import { useEffect, useRef, useCallback, useState } from 'react';

export interface WSMessage {
  type: string;
  content?: string;
  message?: string;
  query?: string;
  sql?: string;
  pool?: { id: number; name: string; sql: string; rows: number };
  pool_id?: number;
  pool_ids?: number[];
  label?: string;
  columns?: string[];
  row_count?: number;
  datasource_id?: number;
  params?: unknown;
}

interface UseWebSocketOptions {
  onMessage: (msg: WSMessage) => void;
  onOpen?: () => void;
  onClose?: () => void;
  onError?: (err: Event) => void;
}

export function useWebSocket({ onMessage, onOpen, onClose, onError }: UseWebSocketOptions) {
  const wsRef = useRef<WebSocket | null>(null);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const reconnectAttemptRef = useRef(0);
  const [isOpen, setIsOpen] = useState(false);
  const handlersRef = useRef({ onMessage, onOpen, onClose, onError });

  // Keep handlers ref current without causing connect to re-create
  handlersRef.current = { onMessage, onOpen, onClose, onError };

  const connect = useCallback(() => {
    const token = localStorage.getItem('token') || '';
    if (!token) return; // Skip connection when not authenticated

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const host = window.location.host;
    const url = `${protocol}//${host}/api/chat?token=${encodeURIComponent(token)}`;

    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      setIsOpen(true);
      reconnectAttemptRef.current = 0; // Reset backoff on successful connection
      handlersRef.current.onOpen?.();
    };

    ws.onmessage = (event) => {
      try {
        const msg: WSMessage = JSON.parse(event.data);
        handlersRef.current.onMessage(msg);
      } catch {
        // Ignore non-JSON messages
      }
    };

    ws.onclose = () => {
      setIsOpen(false);
      handlersRef.current.onClose?.();
      // Exponential backoff: 1s → 2s → 4s → 8s → ... → max 30s
      const delay = Math.min(1000 * Math.pow(2, reconnectAttemptRef.current), 30000);
      reconnectAttemptRef.current += 1;
      reconnectTimer.current = setTimeout(connect, delay);
    };

    ws.onerror = (err) => {
      setIsOpen(false);
      handlersRef.current.onError?.(err);
    };
  }, []); // Stable reference — handlers accessed via ref, no dependency on props

  useEffect(() => {
    connect();
    return () => {
      clearTimeout(reconnectTimer.current);
      const ws = wsRef.current;
      if (ws) {
        // If still connecting, wait for open/close before closing to avoid
        // the "WebSocket is closed before the connection is established" warning.
        if (ws.readyState === WebSocket.CONNECTING) {
          ws.onopen = () => ws.close();
          ws.onerror = () => {};
        } else if (ws.readyState === WebSocket.OPEN) {
          ws.close();
        }
      }
    };
  }, [connect]);

  const send = useCallback((data: Record<string, unknown>) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(data));
    }
  }, []);

  return { send, isOpen };
}
