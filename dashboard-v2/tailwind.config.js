/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,ts,jsx,tsx}'],
  theme: {
    extend: {
      colors: {
        slate: { 950: '#020617', 900: '#0f172a', 800: '#1e293b', 700: '#334155' },
        emerald: { 500: '#10b981' },
        amber: { 500: '#f59e0b' },
        rose: { 500: '#f43f5e' },
      },
      fontFamily: {
        mono: ['JetBrains Mono', 'ui-monospace', 'SFMono-Regular', 'monospace'],
        sans: ['Inter', 'Geist', 'system-ui', 'sans-serif'],
      },
    },
  },
  plugins: [],
}
