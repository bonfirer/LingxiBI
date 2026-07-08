import { useState, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';

const BASE = '/api';

// ── Aurora + Mesh Gradient Background ──
function AuroraBackground() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let animationId: number;
    let width = window.innerWidth;
    let height = window.innerHeight;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = width * dpr;
    canvas.height = height * dpr;
    canvas.style.width = width + 'px';
    canvas.style.height = height + 'px';
    ctx.scale(dpr, dpr);

    let time = 0;

    // Respect reduced-motion: skip the perpetual animation loop entirely.
    const reduceMotion =
      typeof window !== 'undefined' &&
      window.matchMedia &&
      window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    // Simplex-like noise via sin combinations
    function noise(x: number, y: number, t: number) {
      return (
        Math.sin(x * 0.01 + t * 0.3) * Math.cos(y * 0.012 + t * 0.2) +
        Math.sin(x * 0.02 - t * 0.1) * Math.cos(y * 0.008 + t * 0.4) +
        Math.sin((x + y) * 0.005 + t * 0.15)
      ) / 3;
    }

    function draw() {
      time += 0.008;
      ctx!.clearRect(0, 0, width, height);

      // Draw aurora ribbons
      for (let ribbon = 0; ribbon < 4; ribbon++) {
        const baseY = height * (0.2 + ribbon * 0.18);
        const hue = 30 + ribbon * 12; // amber-gold range
        const saturation = 85 - ribbon * 10;

        ctx!.beginPath();
        ctx!.moveTo(0, height);

        for (let x = 0; x <= width; x += 3) {
          const n = noise(x + ribbon * 200, baseY, time + ribbon * 0.5);
          const y = baseY + n * 120 + Math.sin(x * 0.003 + time * 0.5 + ribbon) * 40;
          ctx!.lineTo(x, y);
        }

        ctx!.lineTo(width, height);
        ctx!.closePath();

        const gradient = ctx!.createLinearGradient(0, baseY - 100, 0, baseY + 150);
        gradient.addColorStop(0, `hsla(${hue}, ${saturation}%, 55%, 0)`);
        gradient.addColorStop(0.3, `hsla(${hue}, ${saturation}%, 50%, ${0.06 - ribbon * 0.012})`);
        gradient.addColorStop(0.6, `hsla(${hue}, ${saturation}%, 45%, ${0.04 - ribbon * 0.008})`);
        gradient.addColorStop(1, `hsla(${hue}, ${saturation}%, 40%, 0)`);
        ctx!.fillStyle = gradient;
        ctx!.fill();
      }

      // Glowing orbs that drift
      for (let i = 0; i < 6; i++) {
        const orbX = width * (0.15 + 0.7 * ((Math.sin(time * 0.3 + i * 1.7) + 1) / 2));
        const orbY = height * (0.2 + 0.6 * ((Math.cos(time * 0.2 + i * 2.3) + 1) / 2));
        const radius = 60 + Math.sin(time + i) * 20;
        const alpha = 0.03 + Math.sin(time * 0.5 + i) * 0.015;

        const grad = ctx!.createRadialGradient(orbX, orbY, 0, orbX, orbY, radius);
        grad.addColorStop(0, `hsla(${35 + i * 8}, 90%, 60%, ${alpha * 2})`);
        grad.addColorStop(0.5, `hsla(${35 + i * 8}, 80%, 50%, ${alpha})`);
        grad.addColorStop(1, `hsla(${35 + i * 8}, 70%, 40%, 0)`);

        ctx!.beginPath();
        ctx!.arc(orbX, orbY, radius, 0, Math.PI * 2);
        ctx!.fillStyle = grad;
        ctx!.fill();
      }

      if (!reduceMotion) animationId = requestAnimationFrame(draw);
    }

    draw();

    const handleResize = () => {
      width = window.innerWidth;
      height = window.innerHeight;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = width + 'px';
      canvas.style.height = height + 'px';
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.scale(dpr, dpr);
    };

    window.addEventListener('resize', handleResize);
    return () => {
      cancelAnimationFrame(animationId);
      window.removeEventListener('resize', handleResize);
    };
  }, []);

  return <canvas ref={canvasRef} className="fixed inset-0 z-0 pointer-events-none" />;
}

// ── Morphing blob SVG ──
function MorphingBlob() {
  return (
    <div className="fixed inset-0 z-0 pointer-events-none flex items-center justify-center">
      <svg viewBox="0 0 600 600" className="w-[700px] h-[700px] opacity-[0.07]">
        <defs>
          <linearGradient id="blob-grad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#f59e0b" />
            <stop offset="50%" stopColor="#d97706" />
            <stop offset="100%" stopColor="#92400e" />
          </linearGradient>
        </defs>
        <path fill="url(#blob-grad)">
          <animate
            attributeName="d"
            dur="12s"
            repeatCount="indefinite"
            values="
              M300,220 C380,220 430,280 430,340 C430,400 380,450 300,450 C220,450 170,400 170,340 C170,280 220,220 300,220 Z;
              M300,200 C400,210 450,270 440,350 C430,430 370,470 290,460 C210,450 160,390 170,310 C180,230 230,190 300,200 Z;
              M310,210 C390,230 460,290 440,370 C420,450 350,470 270,450 C190,430 150,370 180,290 C210,210 250,190 310,210 Z;
              M300,220 C380,220 430,280 430,340 C430,400 380,450 300,450 C220,450 170,400 170,340 C170,280 220,220 300,220 Z
            "
          />
        </path>
      </svg>
    </div>
  );
}

export default function LoginPage({ onLogin }: { onLogin: () => void }) {
  const { t } = useTranslation();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [isSetup, setIsSetup] = useState(false);
  const [checking, setChecking] = useState(true);
  const [focused, setFocused] = useState<'user' | 'pass' | null>(null);
  const [expiredNotice, setExpiredNotice] = useState(false);
  const [githubEnabled, setGithubEnabled] = useState(false);

  useEffect(() => {
    // Check if GitHub OAuth is available
    fetch(`${BASE}/auth/github/enabled`).then((r) => r.json()).then((d) => setGithubEnabled(!!d.enabled)).catch(() => {});
  }, []);

  useEffect(() => {
    if (sessionStorage.getItem('sessionExpired')) {
      setExpiredNotice(true);
      sessionStorage.removeItem('sessionExpired');
    }
  }, []);

  // The login screen is a dark, branded full-screen experience with no theme
  // toggle. Force dark mode while it's mounted so light-theme users don't see
  // washed-out white-on-white text, then restore their preference on unmount.
  useEffect(() => {
    const root = document.documentElement;
    const wasLight = root.classList.contains('light');
    root.classList.remove('light');
    root.classList.add('dark');
    return () => {
      if (wasLight) {
        root.classList.add('light');
        root.classList.remove('dark');
      }
    };
  }, []);

  useEffect(() => {
    fetch(`${BASE}/auth/check`)
      .then((r) => r.json())
      .then((data) => { setIsSetup(!data.has_users); })
      .catch(() => {})
      .finally(() => setChecking(false));
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!username.trim() || !password.trim()) return;
    setError('');
    setLoading(true);

    try {
      const endpoint = isSetup ? '/auth/register' : '/auth/login';
      const res = await fetch(`${BASE}${endpoint}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username: username.trim(), password: password.trim() }),
      });

      if (!res.ok) {
        const errText = await res.text();
        setError(errText || t('login.failed'));
        setLoading(false);
        return;
      }

      if (isSetup) {
        const loginRes = await fetch(`${BASE}/auth/login`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ username: username.trim(), password: password.trim() }),
        });
        if (loginRes.ok) {
          const data = await loginRes.json();
          localStorage.setItem('token', data.token);
          localStorage.setItem('user', JSON.stringify({ id: data.id, username: data.username, display_name: data.display_name, role: data.role }));
          onLogin();
        }
      } else {
        const data = await res.json();
        localStorage.setItem('token', data.token);
        localStorage.setItem('user', JSON.stringify({ id: data.id, username: data.username, display_name: data.display_name, role: data.role }));
        onLogin();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : t('login.failed'));
    } finally {
      setLoading(false);
    }
  };

  if (checking) {
    return (
      <div className="min-h-screen bg-obsidian-950 flex items-center justify-center">
        <div className="w-5 h-5 border-2 border-amber-500/30 border-t-amber-500 rounded-full animate-spin" />
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-obsidian-950 flex items-center justify-center px-4 relative overflow-hidden">
      {/* Background effects */}
      <AuroraBackground />
      <MorphingBlob />

      {/* Radial vignette */}
      <div className="fixed inset-0 z-0 pointer-events-none" style={{
        background: 'radial-gradient(ellipse at center, transparent 30%, rgba(8,8,12,0.85) 100%)',
      }} />

      {/* Content */}
      <div className="w-full max-w-sm relative z-10">
        {/* Logo with glow */}
        <div className="text-center mb-8">
          <div className="relative inline-flex items-center justify-center">
            <div className="absolute w-16 h-16 rounded-full bg-amber-500/10 blur-2xl animate-pulse" />
            <div className="relative w-14 h-14 rounded-2xl bg-gradient-to-br from-amber-400 to-amber-600 flex items-center justify-center shadow-[0_0_30px_rgba(245,158,11,0.3)] overflow-hidden">
              <img src="/logo.png" alt="LingxiBI" className="w-full h-full object-contain" />
            </div>
          </div>
          <h1 className="text-2xl font-bold text-gray-50 mt-5 tracking-tight">LingxiBI</h1>
          <p className="text-[11px] text-gray-500 mt-1.5">
            {isSetup ? t('login.setupHint') : t('login.hint')}
          </p>
        </div>

        {/* Glass card */}
        <form
          onSubmit={handleSubmit}
          className="relative rounded-2xl p-6 space-y-4 border border-white/[0.06] bg-obsidian-900/50 backdrop-blur-2xl shadow-[0_8px_60px_-16px_rgba(245,158,11,0.1),inset_0_1px_0_rgba(255,255,255,0.03)]"
        >
          {/* Animated border glow on focus */}
          <div
            className="absolute -inset-[1px] rounded-2xl transition-opacity duration-500 pointer-events-none"
            style={{
              opacity: focused ? 0.6 : 0,
              background: 'conic-gradient(from 180deg, transparent, rgba(245,158,11,0.15), transparent, rgba(245,158,11,0.08), transparent)',
            }}
          />

          <div className="relative">
            <label className="text-[11px] font-medium text-gray-400 block mb-1.5">{t('login.username')}</label>
            <input
              type="text"
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              onFocus={() => setFocused('user')}
              onBlur={() => setFocused(null)}
              autoFocus
              className="w-full bg-white/[0.03] border border-white/[0.08] rounded-xl px-3.5 py-2.5 text-sm text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/40 focus:bg-amber-500/[0.02] focus:shadow-[0_0_0_4px_rgba(245,158,11,0.04)] transition-all duration-300"
              placeholder={isSetup ? t('login.usernamePlaceholderSetup') : t('login.usernamePlaceholder')}
            />
          </div>
          <div className="relative">
            <label className="text-[11px] font-medium text-gray-400 block mb-1.5">{t('login.password')}</label>
            <input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              onFocus={() => setFocused('pass')}
              onBlur={() => setFocused(null)}
              className="w-full bg-white/[0.03] border border-white/[0.08] rounded-xl px-3.5 py-2.5 text-sm text-gray-200 placeholder-gray-600 focus:outline-none focus:border-amber-500/40 focus:bg-amber-500/[0.02] focus:shadow-[0_0_0_4px_rgba(245,158,11,0.04)] transition-all duration-300"
              placeholder="••••••••"
            />
          </div>

          {expiredNotice && !error && (
            <p className="text-[11px] text-amber-400 bg-amber-500/10 border border-amber-500/20 rounded-lg px-3 py-2">{t('login.sessionExpired')}</p>
          )}

          {error && (
            <p className="text-[11px] text-red-400 bg-red-500/10 border border-red-500/20 rounded-lg px-3 py-2">{error}</p>
          )}

          <button
            type="submit"
            disabled={loading || !username.trim() || !password.trim()}
            className="relative w-full overflow-hidden bg-gradient-to-r from-amber-500 to-amber-400 text-[#08080c] font-semibold text-sm py-2.5 rounded-xl transition-all duration-300 active:scale-[0.98] disabled:opacity-50 shadow-[0_4px_24px_-6px_rgba(245,158,11,0.4)] hover:shadow-[0_6px_32px_-6px_rgba(245,158,11,0.55)] hover:brightness-110"
          >
            {/* Button shimmer */}
            <div className="absolute inset-0 bg-gradient-to-r from-transparent via-white/20 to-transparent -translate-x-full hover:translate-x-full transition-transform duration-700" />
            <span className="relative">
              {loading ? t('login.loading') : isSetup ? t('login.createAccount') : t('login.signIn')}
            </span>
          </button>

          {githubEnabled && !isSetup && (
            <>
              <div className="flex items-center gap-3 my-3">
                <div className="flex-1 h-px bg-white/[0.06]" />
                <span className="text-[10px] text-gray-600">{t('login.or')}</span>
                <div className="flex-1 h-px bg-white/[0.06]" />
              </div>
              <button
                type="button"
                onClick={async () => {
                  try {
                    const res = await fetch(`${BASE}/auth/github`);
                    const data = await res.json();
                    if (data.url) window.location.href = data.url;
                  } catch { /* ignore */ }
                }}
                className="w-full flex items-center justify-center gap-2 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-gray-200 text-sm py-2.5 rounded-xl transition-all duration-300 active:scale-[0.98]"
              >
                <svg width="18" height="18" viewBox="0 0 24 24" fill="currentColor">
                  <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12"/>
                </svg>
                {t('login.githubSignIn')}
              </button>
            </>
          )}
        </form>

        <p className="text-center text-[9px] text-gray-700 mt-6 tracking-wide">
          Powered by AI · Marco ·{' '}
          <a
            href="mailto:macrogroot@outlook.com"
            className="text-gray-600 hover:text-amber-500/80 transition-colors"
          >
            macrogroot@outlook.com
          </a>
        </p>
      </div>
    </div>
  );
}
