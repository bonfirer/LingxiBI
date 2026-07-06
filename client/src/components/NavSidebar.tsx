import { NavLink, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { useState, useEffect } from 'react';
import {
  Database,
  ChatCircle,
  ChartBar,
  Star,
  Gear,
  Sun,
  Moon,
  ClockCounterClockwise,
  Trophy,
  SignOut,
  Camera,
  Bell,
  Sparkle,
  GithubLogo,
} from '@phosphor-icons/react';
import { useUIStore } from '../stores/uiStore';

const navKeys = [
  { to: '/datasources', icon: Database, tKey: 'nav.datasources' },
  { to: '/conversations', icon: ChatCircle, tKey: 'nav.conversations' },
  { to: '/reports', icon: ChartBar, tKey: 'nav.reports' },
  { to: '/metrics', icon: Star, tKey: 'nav.metrics' },
  { to: '/snapshots', icon: Camera, tKey: 'nav.snapshots' },
  { to: '/alerts', icon: Bell, tKey: 'nav.alerts' },
  { to: '/logs', icon: ClockCounterClockwise, tKey: 'nav.logs' },
  { to: '/wishes', icon: Sparkle, tKey: 'nav.wishes' },
];

export default function NavSidebar() {
  const { t, i18n } = useTranslation();
  const location = useLocation();
  const { theme, toggleTheme } = useUIStore();

  const toggleLang = () => {
    const next = i18n.language === 'zh' ? 'en' : 'zh';
    i18n.changeLanguage(next);
  };

  return (
    <nav aria-label={t('nav.mainNav')} className="w-[52px] bg-obsidian-950 border-r border-obsidian-700 flex flex-col items-center py-3 gap-1 flex-shrink-0">
      {/* Logo */}
      <div className="w-8 h-8 rounded-lg flex items-center justify-center mb-4 flex-shrink-0 overflow-hidden">
        <img src="/logo.png" alt="LingxiBI" className="w-full h-full object-contain" />
      </div>

      {/* Nav Items */}
      {navKeys.map(({ to, icon: Icon, tKey }) => {
        const isActive = location.pathname.startsWith(to);
        return (
          <NavLink
            key={to}
            to={to}
            title={t(tKey)}
            aria-label={t(tKey)}
            className={`
              w-9 h-9 rounded-md flex items-center justify-center transition-premium
              ${isActive
                ? 'bg-amber-500/10 border-l-2 border-amber-500 text-amber-500'
                : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'}
            `}
          >
            <Icon size={18} weight={isActive ? 'fill' : 'regular'} />
          </NavLink>
        );
      })}

      {/* Spacer */}
      <div className="flex-1" />

      {/* Settings */}
      <NavLink
        to="/settings"
        title={t('nav.settings')}
        aria-label={t('nav.settings')}
        className={`
          w-9 h-9 rounded-md flex items-center justify-center transition-premium
          ${location.pathname.startsWith('/settings')
            ? 'bg-amber-500/10 border-l-2 border-amber-500 text-amber-500'
            : 'text-gray-400 hover:text-gray-200 hover:bg-obsidian-800'}
        `}
      >
        <Gear size={18} weight={location.pathname.startsWith('/settings') ? 'fill' : 'regular'} />
      </NavLink>

      {/* Star on GitHub */}
      <a
        href="https://github.com/bonfirer/LingxiBI"
        target="_blank"
        rel="noreferrer"
        title={t('nav.starOnGithub')}
        aria-label={t('nav.starOnGithub')}
        className="w-9 h-9 rounded-md flex items-center justify-center text-gray-400 hover:text-amber-500 hover:bg-obsidian-800 transition-premium"
      >
        <GithubLogo size={18} />
      </a>

      {/* Language Switcher */}
      <button
        onClick={toggleLang}
        className="w-7 h-7 text-[10px] text-gray-500 hover:text-gray-300 hover:bg-obsidian-800 rounded-md transition-premium font-medium"
        title={i18n.language === 'zh' ? 'Switch to English' : '切换到中文'}
        aria-label={i18n.language === 'zh' ? 'Switch to English' : '切换到中文'}
      >
        {i18n.language === 'zh' ? 'EN' : '中'}
      </button>

      {/* Theme Toggle */}
      <button
        onClick={toggleTheme}
        className="w-7 h-7 flex items-center justify-center text-gray-500 hover:text-gray-300 hover:bg-obsidian-800 rounded-md transition-premium"
        title={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
        aria-label={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
      >
        {theme === 'dark' ? <Sun size={14} /> : <Moon size={14} />}
      </button>

      {/* Logout */}
      <button
        onClick={() => {
          localStorage.removeItem('token');
          localStorage.removeItem('user');
          window.location.reload();
        }}
        className="w-7 h-7 flex items-center justify-center text-gray-500 hover:text-red-400 hover:bg-obsidian-800 rounded-md transition-premium"
        title={t('nav.logout')}
        aria-label={t('nav.logout')}
      >
        <SignOut size={14} />
      </button>

      {/* User Avatar + Achievements */}
      <AchievementsBadge />
    </nav>
  );
}

// ── Achievement definitions ──
const ACHIEVEMENT_DEFS: Record<string, { emoji: string; label: string }> = {
  first_report: { emoji: '📊', label: '初次创建报表' },
  report_five: { emoji: '📈', label: '创建 5 份报表' },
  report_ten: { emoji: '🏆', label: '创建 10 份报表' },
  first_publish: { emoji: '🚀', label: '首次发布报表' },
  first_share: { emoji: '🔗', label: '首次分享报表' },
  metric_collector: { emoji: '⭐', label: '积累 5 个指标' },
  metric_master: { emoji: '💫', label: '指标大师(20+)' },
  knowledge_seeker: { emoji: '📚', label: '知识探索者(5条)' },
  knowledge_sage: { emoji: '🧠', label: '知识贤者(20条)' },
  ai_trainer: { emoji: '🤖', label: 'AI 训练师(5例)' },
  ai_master: { emoji: '🎓', label: 'AI 大师(20例)' },
  chatterbox: { emoji: '💬', label: '话痨(10次对话)' },
  data_explorer: { emoji: '🗺️', label: '数据探险家(50次)' },
  style_explorer: { emoji: '🎨', label: '风格探索者(3种)' },
  fashionista: { emoji: '👗', label: '时尚达人(8种风格)' },
};

function AchievementsBadge() {
  const [show, setShow] = useState(false);
  const [achievements, setAchievements] = useState<{ achievement: string; unlocked_at: string }[]>([]);

  useEffect(() => {
    fetch('/api/achievements', {
      headers: { Authorization: `Bearer ${localStorage.getItem('token') || ''}` },
    })
      .then((r) => r.json())
      .then(setAchievements)
      .catch(() => {});
  }, [show]);

  return (
    <div className="relative mt-1">
      <button
        onClick={() => setShow(!show)}
        className="w-8 h-8 rounded-full bg-amber-500 flex items-center justify-center flex-shrink-0 hover:scale-110 transition-transform"
        title="成就"
      >
        {achievements.length > 0 ? (
          <Trophy size={14} className="text-[#08080c]" weight="fill" />
        ) : (
          <span className="text-[#08080c] text-[10px] font-bold">
            {JSON.parse(localStorage.getItem('user') || '{}').username?.[0]?.toUpperCase() || 'U'}
          </span>
        )}
      </button>
      {achievements.length > 0 && (
        <span className="absolute -top-0.5 -right-0.5 w-3.5 h-3.5 bg-amber-500 border-2 border-obsidian-950 rounded-full flex items-center justify-center">
          <span className="text-[7px] text-[#08080c] font-bold">{achievements.length}</span>
        </span>
      )}

      {show && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setShow(false)} />
          <div className="absolute bottom-full left-12 mb-2 z-50 bg-obsidian-900 border border-obsidian-700 rounded-xl shadow-2xl w-64 max-h-80 overflow-hidden">
            <div className="px-3 py-2 border-b border-obsidian-700 flex items-center gap-2">
              <Trophy size={14} className="text-amber-500" weight="fill" />
              <span className="text-[11px] font-semibold text-gray-200">成就 ({achievements.length}/{Object.keys(ACHIEVEMENT_DEFS).length})</span>
            </div>
            <div className="p-2 overflow-y-auto max-h-60 scrollbar-thin space-y-0.5">
              {Object.entries(ACHIEVEMENT_DEFS).map(([key, def]) => {
                const unlocked = achievements.find((a) => a.achievement === key);
                return (
                  <div key={key} className={`flex items-center gap-2 px-2 py-1.5 rounded-lg ${unlocked ? 'bg-amber-500/5' : 'opacity-40'}`}>
                    <span className="text-sm">{def.emoji}</span>
                    <span className={`text-[10px] flex-1 ${unlocked ? 'text-gray-200' : 'text-gray-600'}`}>{def.label}</span>
                    {unlocked && <span className="text-[8px] text-gray-500">✓</span>}
                  </div>
                );
              })}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
