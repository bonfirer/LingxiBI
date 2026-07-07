import { type ReactNode, Component, useEffect, useRef, useState } from 'react';
import { X, WarningCircle } from '@phosphor-icons/react';

// ── Error Boundary ──
interface ErrorBoundaryProps { children: ReactNode; }
interface ErrorBoundaryState { hasError: boolean; error: Error | null; }

export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="h-dvh w-full flex items-center justify-center bg-obsidian-950">
          <div className="flex flex-col items-center gap-4 max-w-md text-center">
            <div className="w-14 h-14 rounded-2xl bg-red-500/10 border border-red-500/20 flex items-center justify-center">
              <WarningCircle size={28} className="text-red-400" />
            </div>
            <h2 className="text-sm font-semibold text-gray-200">Something went wrong</h2>
            <p className="text-xs text-gray-500 leading-relaxed">
              {this.state.error?.message || 'An unexpected error occurred.'}
            </p>
            <button
              onClick={() => window.location.reload()}
              className="bg-amber-500 hover:bg-amber-400 text-[#08080c] font-semibold text-xs px-4 py-2 rounded-lg transition-all active:translate-y-[1px]"
            >
              Reload Page
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

// ── Error Banner ──
export function ErrorBanner({
  message,
  onDismiss,
}: {
  message: string;
  onDismiss?: () => void;
}) {
  return (
    <div className="bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 mb-4 flex items-center justify-between">
      <span className="text-xs text-red-400">{message}</span>
      {onDismiss && (
        <button onClick={onDismiss} className="text-red-400 hover:text-red-300">
          <X size={14} />
        </button>
      )}
    </div>
  );
}

// ── Loading Skeleton ──
export function LoadingSkeleton({
  rows = 3,
  className = '',
}: {
  rows?: number;
  className?: string;
}) {
  return (
    <div className={`space-y-2 ${className}`}>
      {Array.from({ length: rows }).map((_, i) => (
        <div
          key={i}
          className="bg-obsidian-900 border border-obsidian-700 rounded-lg p-4 animate-pulse"
        >
          <div className="flex items-center gap-3">
            <div className="w-2 h-2 rounded-full bg-obsidian-700" />
            <div className="flex-1 space-y-2">
              <div className="h-3 bg-obsidian-700 rounded w-32" />
              <div className="h-2 bg-obsidian-700 rounded w-20" />
            </div>
            <div className="flex gap-2">
              <div className="h-6 w-16 bg-obsidian-700 rounded" />
              <div className="h-6 w-20 bg-obsidian-700 rounded" />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

// ── Empty State ──
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon: React.ComponentType<{ size: number; className?: string }>;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center py-24 text-center">
      <div className="w-14 h-14 rounded-2xl bg-obsidian-800 border border-obsidian-700 flex items-center justify-center mb-4">
        <Icon size={26} className="text-gray-600" />
      </div>
      <h2 className="text-sm font-semibold text-gray-300 mb-1">{title}</h2>
      <p className="text-xs text-gray-600 max-w-[280px] mb-4">{description}</p>
      {action}
    </div>
  );
}

// ── Page Header ──
export function PageHeader({
  title,
  description,
  action,
}: {
  title: string;
  description?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between mb-6">
      <div>
        <h1 className="text-lg font-bold text-gray-100 tracking-tight">{title}</h1>
        {description && <div className="text-xs text-gray-500 mt-1">{description}</div>}
      </div>
      {action}
    </div>
  );
}

// ── Status Dot ──
export function StatusDot({ status }: { status: string }) {
  const color =
    status === 'connected'
      ? 'bg-data-green'
      : status === 'error'
        ? 'bg-red-500'
        : 'bg-data-amber';
  return <span className={`inline-block w-2 h-2 rounded-full ${color}`} />;
}

// ── Card ──
export function Card({
  children,
  className = '',
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`bg-obsidian-900 border border-obsidian-700 rounded-xl ${className}`}
    >
      {children}
    </div>
  );
}

// ── Confirm Dialog ──
// Accessible, reusable confirmation modal: closes on Escape or backdrop click,
// focuses the confirm button on open, and disables both buttons while the
// (possibly async) confirm handler is in flight to prevent double-submits.
export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  cancelLabel,
  onConfirm,
  onCancel,
  danger = true,
}: {
  open: boolean;
  title: string;
  message?: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
  danger?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const confirmRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) onCancel();
    };
    window.addEventListener('keydown', onKey);
    const raf = requestAnimationFrame(() => confirmRef.current?.focus());
    return () => {
      window.removeEventListener('keydown', onKey);
      cancelAnimationFrame(raf);
    };
  }, [open, busy, onCancel]);

  if (!open) return null;

  const handleConfirm = async () => {
    try {
      setBusy(true);
      await onConfirm();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={() => { if (!busy) onCancel(); }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="bg-obsidian-900 border border-obsidian-700 rounded-xl p-5 w-80 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-sm font-semibold text-gray-100 mb-2">{title}</h3>
        {message && <p className="text-xs text-gray-400 mb-4">{message}</p>}
        <div className="flex items-center justify-end gap-2">
          <button
            onClick={onCancel}
            disabled={busy}
            className="text-xs text-gray-400 hover:text-gray-200 hover:bg-obsidian-700 px-3 py-1.5 rounded-md border border-obsidian-700 transition-premium disabled:opacity-50"
          >
            {cancelLabel}
          </button>
          <button
            ref={confirmRef}
            onClick={handleConfirm}
            disabled={busy}
            className={`text-xs px-3 py-1.5 rounded-md transition-premium disabled:opacity-60 ${
              danger
                ? 'text-white bg-red-600 hover:bg-red-500'
                : 'text-[#08080c] bg-amber-500 hover:bg-amber-400'
            }`}
          >
            {busy ? '…' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
