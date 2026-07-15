/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        obsidian: {
          950: 'rgb(var(--obsidian-950) / <alpha-value>)',
          900: 'rgb(var(--obsidian-900) / <alpha-value>)',
          800: 'rgb(var(--obsidian-800) / <alpha-value>)',
          700: 'rgb(var(--obsidian-700) / <alpha-value>)',
        },
        amber: {
          500: '#d4a853',
        },
        'data-green': '#4ade80',
        'data-amber': '#f59e0b',
      },
      fontFamily: {
        sans: ['AlibabaPuHuiTi', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'Geist Mono', 'ui-monospace', 'monospace'],
      },
      transitionTimingFunction: {
        'premium': 'cubic-bezier(0.16, 1, 0.3, 1)',
      },
    },
  },
  plugins: [],
}

