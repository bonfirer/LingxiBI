import { Outlet, useLocation } from 'react-router-dom';
import NavSidebar from './NavSidebar';
import AssetPanel from './AssetPanel';
import AIPanel from './AIPanel';

export default function Layout() {
  const location = useLocation();
  const isConversationsPage = location.pathname.startsWith('/conversations');
  const isReportDetail = /^\/reports\/\d+/.test(location.pathname);
  const isReportsPage = location.pathname.startsWith('/reports');
  const isLogsPage = location.pathname.startsWith('/logs');
  const isSettingsPage = location.pathname.startsWith('/settings');
  const isSnapshotsPage = location.pathname.startsWith('/snapshots');
  const isAlertsPage = location.pathname.startsWith('/alerts');
  const isWishesPage = location.pathname.startsWith('/wishes');
  const hideAssetPanel = isConversationsPage || isReportDetail || isLogsPage || isSettingsPage || isSnapshotsPage || isAlertsPage || isWishesPage;
  // Hide the AI assistant on conversation, report detail, report list, logs, settings, snapshots, alerts, and wishes pages
  const hideAIPanel = isConversationsPage || isReportDetail || isReportsPage || isLogsPage || isSettingsPage || isSnapshotsPage || isAlertsPage || isWishesPage;

  return (
    <div className="flex h-dvh w-full overflow-hidden bg-obsidian-950 md:flex-row flex-col">
      <NavSidebar />
      {!hideAssetPanel && <AssetPanel />}
      <main className="flex-1 overflow-y-auto scrollbar-thin bg-obsidian-950 min-h-0 pb-16 md:pb-0">
        <Outlet />
      </main>
      {!hideAIPanel && (
        <div className="hidden md:block h-full">
          <AIPanel />
        </div>
      )}
    </div>
  );
}
