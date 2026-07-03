import { lazy, Suspense, useState, useEffect } from 'react';
import { I18nextProvider } from 'react-i18next';
import i18n from './i18n';
import { Routes, Route, Navigate } from 'react-router-dom';
import LLMConfigProvider from './components/LLMConfigProvider';
import Layout from './components/Layout';
import LoginPage from './pages/LoginPage';
import EasterEgg from './components/EasterEgg';
import Toaster from './components/Toaster';
import { Sparkle } from '@phosphor-icons/react';

// Lazy-load heavy pages for faster initial load
const DataSourcesPage = lazy(() => import('./pages/DataSourcesPage'));
const ConversationsPage = lazy(() => import('./pages/ConversationsPage'));
const ReportsPage = lazy(() => import('./pages/ReportsPage'));
const ReportDetailPage = lazy(() => import('./pages/ReportDetailPage'));
const MetricsPage = lazy(() => import('./pages/MetricsPage'));
const SnapshotsPage = lazy(() => import('./pages/SnapshotsPage'));
const AlertsPage = lazy(() => import('./pages/AlertsPage'));
const LogsPage = lazy(() => import('./pages/LogsPage'));
const SettingsPage = lazy(() => import('./pages/SettingsPage'));
const SharedReportPage = lazy(() => import('./pages/SharedReportPage'));

function PageLoader() {
  return (
    <div className="flex items-center justify-center h-full">
      <Sparkle size={20} className="text-amber-500/40 animate-pulse" />
    </div>
  );
}

export default function App() {
  const [authed, setAuthed] = useState<boolean | null>(null); // null = checking

  useEffect(() => {
    // Handle GitHub OAuth redirect: pick up the token from the query string.
    const params = new URLSearchParams(window.location.search);
    const ghToken = params.get('github_token');
    if (ghToken) {
      localStorage.setItem('token', ghToken);
      const userId = params.get('user_id');
      const username = params.get('username') || '';
      const role = params.get('role') || 'member';
      localStorage.setItem('user', JSON.stringify({ id: Number(userId), username, role }));
      // Clean the URL so the token doesn't linger in browser history.
      window.history.replaceState({}, '', '/');
      setAuthed(true);
      return;
    }

    const token = localStorage.getItem('token');
    if (!token) { setAuthed(false); return; }
    // Validate token and cache the current user (incl. role) for UI gating.
    fetch('/api/auth/me', { headers: { Authorization: `Bearer ${token}` } })
      .then(async (r) => {
        setAuthed(r.ok);
        if (r.ok) {
          try {
            const me = await r.json();
            localStorage.setItem('user', JSON.stringify(me));
          } catch { /* ignore */ }
        } else {
          localStorage.removeItem('token');
        }
      })
      .catch(() => setAuthed(false));
  }, []);

  // Drop to the login screen when any API call reports an expired session.
  useEffect(() => {
    const onExpired = () => setAuthed(false);
    window.addEventListener('auth-expired', onExpired);
    return () => window.removeEventListener('auth-expired', onExpired);
  }, []);

  // Still checking
  if (authed === null) {
    return (
      <div className="min-h-screen bg-obsidian-950 flex items-center justify-center">
        <div className="w-5 h-5 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
      </div>
    );
  }

  return (
    <I18nextProvider i18n={i18n}>
      <EasterEgg />
      <Toaster />
      <Suspense fallback={<PageLoader />}>
        <Routes>
          {/* Public share route — no auth required */}
          <Route path="/share/:token" element={<SharedReportPage />} />

          {/* Auth check */}
          {!authed ? (
            <Route path="*" element={<LoginPage onLogin={() => setAuthed(true)} />} />
          ) : (
            <Route element={<LLMConfigProvider><Layout /></LLMConfigProvider>}>
              <Route path="/" element={<Navigate to="/reports" replace />} />
              <Route path="/datasources" element={<DataSourcesPage />} />
              <Route path="/knowledge-graph" element={<Navigate to="/datasources" replace />} />
              <Route path="/conversations" element={<ConversationsPage />} />
              <Route path="/reports" element={<ReportsPage />} />
              <Route path="/reports/:id" element={<ReportDetailPage />} />
              <Route path="/metrics" element={<MetricsPage />} />
              <Route path="/snapshots" element={<SnapshotsPage />} />
              <Route path="/alerts" element={<AlertsPage />} />
              <Route path="/logs" element={<LogsPage />} />
              <Route path="/settings" element={<SettingsPage />} />
            </Route>
          )}
        </Routes>
      </Suspense>
    </I18nextProvider>
  );
}
