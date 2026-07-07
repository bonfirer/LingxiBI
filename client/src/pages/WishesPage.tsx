import { useEffect, useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Sparkle, Trash, Plus, CheckCircle, XCircle, Clock, Spinner } from '@phosphor-icons/react';
import { PageHeader, EmptyState, Card, ConfirmDialog } from '../components/ui';
import { wishesApi } from '../lib/api';
import type { Wish } from '../lib/types';

const CATEGORIES = ['feature', 'bug', 'improvement', 'other'];
const STATUSES = ['pending', 'accepted', 'rejected', 'done'];

export default function WishesPage() {
  const { t } = useTranslation();
  const [wishes, setWishes] = useState<Wish[]>([]);
  const [loading, setLoading] = useState(true);
  const [submitting, setSubmitting] = useState(false);
  const [deletingId, setDeletingId] = useState<number | null>(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<number | null>(null);
  const [form, setForm] = useState({ title: '', content: '', category: 'feature' });
  const [error, setError] = useState<string | null>(null);
  const [isAdmin, setIsAdmin] = useState(false);

  useEffect(() => {
    try {
      const user = JSON.parse(localStorage.getItem('user') || '{}');
      setIsAdmin(user.role === 'admin');
    } catch { /* ignore */ }
  }, []);

  const fetchWishes = useCallback(async () => {
    setLoading(true);
    try {
      const data = await wishesApi.list();
      setWishes(data);
    } catch (e) {
      setError(t('errors.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    fetchWishes();
  }, [fetchWishes]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const title = form.title.trim();
    const content = form.content.trim();
    if (!title || !content) return;

    setSubmitting(true);
    setError(null);
    try {
      await wishesApi.create({ title, content, category: form.category });
      setForm({ title: '', content: '', category: 'feature' });
      await fetchWishes();
    } catch (e) {
      setError(t('errors.createFailed'));
    } finally {
      setSubmitting(false);
    }
  };

  const handleDelete = async (id: number) => {
    setDeletingId(id);
    try {
      await wishesApi.delete(id);
      setWishes((prev) => prev.filter((w) => w.id !== id));
    } catch {
      setError(t('errors.deleteFailed'));
    } finally {
      setDeletingId(null);
      setConfirmDeleteId(null);
    }
  };

  const handleStatusChange = async (wish: Wish, status: string) => {
    try {
      const updated = await wishesApi.update(wish.id, { status });
      setWishes((prev) => prev.map((w) => (w.id === wish.id ? updated : w)));
    } catch {
      setError(t('errors.saveFailed'));
    }
  };

  return (
    <div className="p-4 md:p-6 w-full max-w-6xl mx-auto">
      <PageHeader
        title={t('wishes.title')}
        description={t('wishes.description')}
      />

      {error && (
        <div className="mb-4 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2 text-xs text-red-400">
          {error}
        </div>
      )}

      <Card className="p-4 mb-6">
        <form onSubmit={handleSubmit} className="space-y-3">
          <div className="flex items-center gap-2 text-amber-500 text-xs font-semibold uppercase tracking-wide">
            <Sparkle size={14} weight="fill" />
            {t('wishes.newWish')}
          </div>
          <div className="grid grid-cols-1 md:grid-cols-4 gap-3">
            <input
              type="text"
              value={form.title}
              onChange={(e) => setForm({ ...form, title: e.target.value })}
              placeholder={t('wishes.titlePlaceholder')}
              maxLength={255}
              className="md:col-span-3 bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder:text-gray-600 focus:outline-none focus:border-amber-500/50"
            />
            <select
              value={form.category}
              onChange={(e) => setForm({ ...form, category: e.target.value })}
              className="bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 focus:outline-none focus:border-amber-500/50"
            >
              {CATEGORIES.map((c) => (
                <option key={c} value={c}>{t(`wishes.category.${c}`)}</option>
              ))}
            </select>
          </div>
          <textarea
            value={form.content}
            onChange={(e) => setForm({ ...form, content: e.target.value })}
            placeholder={t('wishes.contentPlaceholder')}
            rows={4}
            className="w-full bg-obsidian-950 border border-obsidian-700 rounded-lg px-3 py-2 text-xs text-gray-200 placeholder:text-gray-600 focus:outline-none focus:border-amber-500/50 resize-none"
          />
          <div className="flex justify-end">
            <button
              type="submit"
              disabled={submitting || !form.title.trim() || !form.content.trim()}
              className="flex items-center gap-1.5 bg-amber-500 hover:bg-amber-400 disabled:opacity-50 disabled:cursor-not-allowed text-[#08080c] text-xs font-semibold px-4 py-2 rounded-lg transition-premium"
            >
              {submitting ? <Spinner size={14} className="animate-spin" /> : <Plus size={14} weight="bold" />}
              {t('wishes.submit')}
            </button>
          </div>
        </form>
      </Card>

      {loading ? (
        <div className="flex items-center gap-2 py-12 justify-center">
          <div className="w-3 h-3 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
          <span className="text-xs text-gray-500">{t('common.loading')}</span>
        </div>
      ) : wishes.length === 0 ? (
        <EmptyState
          icon={Sparkle}
          title={t('wishes.empty.title')}
          description={t('wishes.empty.description')}
        />
      ) : (
        <div className="grid grid-cols-1 xl:grid-cols-2 gap-3">
          {wishes.map((wish) => (
            <Card key={wish.id} className="p-4">
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 flex-wrap mb-1.5">
                    <h3 className="text-sm font-semibold text-gray-200">{wish.title}</h3>
                    <StatusBadge status={wish.status} t={t} />
                    <CategoryBadge category={wish.category} t={t} />
                  </div>
                  <p className="text-xs text-gray-400 whitespace-pre-wrap">{wish.content}</p>
                  <p className="text-[10px] text-gray-600 mt-2">
                    {formatTime(wish.created_at)}
                  </p>
                </div>
                <div className="flex items-center gap-2 flex-shrink-0">
                  {isAdmin && (
                    <select
                      value={wish.status}
                      onChange={(e) => handleStatusChange(wish, e.target.value)}
                      className="bg-obsidian-950 border border-obsidian-700 rounded-lg px-2 py-1.5 text-[10px] text-gray-200 focus:outline-none focus:border-amber-500/50"
                    >
                      {STATUSES.map((s) => (
                        <option key={s} value={s}>{t(`wishes.status.${s}`)}</option>
                      ))}
                    </select>
                  )}
                  <button
                    onClick={() => setConfirmDeleteId(wish.id)}
                    disabled={deletingId === wish.id}
                    className="p-1.5 text-gray-500 hover:text-red-400 hover:bg-obsidian-800 rounded-md transition-premium disabled:opacity-50"
                    title={t('common.delete')}
                  >
                    {deletingId === wish.id ? (
                      <Spinner size={14} className="animate-spin" />
                    ) : (
                      <Trash size={14} />
                    )}
                  </button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <ConfirmDialog
        open={confirmDeleteId !== null}
        title={t('wishes.deleteConfirm.title')}
        message={t('wishes.deleteConfirm.message')}
        confirmLabel={t('common.delete')}
        cancelLabel={t('common.cancel')}
        danger
        onConfirm={() => { if (confirmDeleteId) handleDelete(confirmDeleteId); }}
        onCancel={() => setConfirmDeleteId(null)}
      />
    </div>
  );
}

function StatusBadge({ status, t }: { status: string; t: (k: string) => string }) {
  const config: Record<string, { icon: React.ElementType; className: string }> = {
    pending: { icon: Clock, className: 'bg-amber-500/10 text-amber-500 border-amber-500/20' },
    accepted: { icon: CheckCircle, className: 'bg-blue-400/10 text-blue-400 border-blue-400/20' },
    rejected: { icon: XCircle, className: 'bg-red-500/10 text-red-400 border-red-500/20' },
    done: { icon: CheckCircle, className: 'bg-data-green/10 text-data-green border-data-green/20' },
  };
  const { icon: Icon, className } = config[status] || config.pending;
  return (
    <span className={`inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium border ${className}`}>
      <Icon size={10} weight="fill" />
      {t(`wishes.status.${status}`)}
    </span>
  );
}

function CategoryBadge({ category, t }: { category: string; t: (k: string) => string }) {
  return (
    <span className="inline-flex items-center px-2 py-0.5 rounded text-[10px] font-medium bg-obsidian-800 text-gray-400 border border-obsidian-700">
      {t(`wishes.category.${category}`)}
    </span>
  );
}

function formatTime(ts?: string) {
  if (!ts) return '';
  return new Date(ts).toLocaleString('zh-CN', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}
