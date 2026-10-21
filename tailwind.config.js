/** @type {import('tailwindcss').Config} */
const v = (name) => `rgb(var(--${name}) / <alpha-value>)`

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        ground: v('ground'),
        well: v('well'),
        deck: v('deck'),
        raised: v('raised'),
        rivet: v('rivet'),
        ink: v('ink'),
        muted: v('muted'),
        faint: v('faint'),
        hop: v('hop'),
        'hop-ink': v('hop-ink'),
        full: v('full'),
        half: v('half'),
        low: v('low'),
        empty: v('empty')
      },
      fontFamily: {
        sans: ['"Hanken Grotesk Variable"', '"Hanken Grotesk"', 'system-ui', '-apple-system', 'Segoe UI', 'sans-serif'],
        serif: ['"Source Serif 4 Variable"', '"Source Serif 4"', 'Georgia', 'serif'],
        mono: ['"IBM Plex Mono"', 'ui-monospace', 'Consolas', 'monospace']
      },
      fontSize: {
        xs: ['12px', '16px'],
        sm: ['13px', '18px'],
        base: ['14px', '20px'],
        md: ['15px', '22px'],
        lg: ['20px', '26px'],
        hero: ['44px', '44px']
      },
      borderRadius: { DEFAULT: '6px', md: '6px', lg: '8px' }
    }
  },
  plugins: [
    // `compact:` styles apply inside a collapsed sidebar.
    ({ addVariant }) => addVariant('compact', '.sidebar-compact &')
  ]
}
