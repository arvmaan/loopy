/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        void: 'rgb(var(--color-void) / <alpha-value>)',
        chrome: 'rgb(var(--color-chrome) / <alpha-value>)',
        card: 'var(--color-card)',
        primary: 'rgb(var(--color-primary) / <alpha-value>)',
        secondary: 'rgb(var(--color-secondary) / <alpha-value>)',
        warning: 'rgb(var(--color-warning) / <alpha-value>)',
        danger: 'rgb(var(--color-danger) / <alpha-value>)',
        border: 'rgb(var(--color-border) / <alpha-value>)',
        dim: 'rgb(var(--color-dim) / <alpha-value>)',
      },
      fontFamily: {
        heading: ['Space Grotesk', 'system-ui', 'sans-serif'],
        body: ['Inter', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'Fira Code', 'monospace'],
      },
      animation: {
        'radar-ping': 'radar-ping 1.5s cubic-bezier(0, 0, 0.2, 1) infinite',
        'pulse-soft': 'pulse-soft 2s ease-in-out infinite',
      },
      keyframes: {
        'radar-ping': {
          '0%': { transform: 'scale(1)', opacity: '0.75' },
          '75%, 100%': { transform: 'scale(2)', opacity: '0' },
        },
        'pulse-soft': {
          '0%, 100%': { opacity: '1' },
          '50%': { opacity: '0.6' },
        },
      },
    },
  },
  plugins: [require('@tailwindcss/typography')],
}
