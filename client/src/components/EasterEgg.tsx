import { useEffect, useRef, useState, useCallback } from 'react';

// Konami code: ↑ ↑ ↓ ↓ ← → ← → B A
const SEQUENCE = [
  'ArrowUp', 'ArrowUp', 'ArrowDown', 'ArrowDown',
  'ArrowLeft', 'ArrowRight', 'ArrowLeft', 'ArrowRight',
  'b', 'a',
];

// A playful full-screen "data rain" of falling numbers in the app's amber
// theme, triggered by the Konami code. Pure canvas, no dependencies.
export default function EasterEgg() {
  const [active, setActive] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const progressRef = useRef(0);

  // Listen for the secret sequence anywhere in the app.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Ignore while typing in inputs so it doesn't fight with normal use.
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.isContentEditable)) {
        progressRef.current = 0;
        return;
      }
      const want = SEQUENCE[progressRef.current];
      if (e.key.toLowerCase() === want.toLowerCase()) {
        progressRef.current += 1;
        if (progressRef.current === SEQUENCE.length) {
          progressRef.current = 0;
          setActive(true);
        }
      } else {
        // Allow a restart if the wrong key is actually the first key.
        progressRef.current = e.key === SEQUENCE[0] ? 1 : 0;
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const stop = useCallback(() => setActive(false), []);

  // Run the rain animation while active.
  useEffect(() => {
    if (!active) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let raf = 0;
    const resize = () => {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    };
    resize();
    window.addEventListener('resize', resize);

    const fontSize = 16;
    const columns = Math.floor(canvas.width / fontSize);
    const drops = new Array(columns).fill(0).map(() => Math.random() * -50);
    const glyphs = '0123456789$%∑∆µ×÷=±<>'.split('');

    const startedAt = Date.now();
    const DURATION = 7000;

    const draw = () => {
      // Fade trail
      ctx.fillStyle = 'rgba(8, 8, 12, 0.12)';
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      ctx.font = `${fontSize}px monospace`;
      for (let i = 0; i < drops.length; i++) {
        const ch = glyphs[Math.floor(Math.random() * glyphs.length)];
        const x = i * fontSize;
        const y = drops[i] * fontSize;
        // Bright leading glyph, amber trail
        ctx.fillStyle = Math.random() > 0.975 ? '#fff7ed' : '#f59e0b';
        ctx.fillText(ch, x, y);
        if (y > canvas.height && Math.random() > 0.975) drops[i] = 0;
        drops[i] += 1;
      }

      if (Date.now() - startedAt > DURATION) {
        stop();
        return;
      }
      raf = requestAnimationFrame(draw);
    };
    draw();

    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') stop(); };
    window.addEventListener('keydown', onKey);

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener('resize', resize);
      window.removeEventListener('keydown', onKey);
    };
  }, [active, stop]);

  if (!active) return null;

  return (
    <div
      onClick={stop}
      className="fixed inset-0 z-[9999] cursor-pointer"
      title="Click or press Esc to dismiss"
    >
      <canvas ref={canvasRef} className="absolute inset-0 w-full h-full" />
      <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
        <div className="text-center select-none">
          <div className="text-amber-400 text-4xl font-bold tracking-widest drop-shadow-[0_0_20px_rgba(245,158,11,0.6)] animate-pulse">
            DATA MODE
          </div>
          <div className="text-amber-200/70 text-xs mt-3 font-mono tracking-wide">
            crafted with curiosity · made by Kiro &amp; Marco
          </div>
        </div>
      </div>
    </div>
  );
}
