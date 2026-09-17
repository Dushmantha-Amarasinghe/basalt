import type { Config } from 'tailwindcss'

/**
 * Design tokens carried over from Frostbyte, so the two apps feel like siblings.
 * Values are deliberately identical; only the accent's name changed from
 * `frost` to `basalt`.
 */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        // Neutral graphite base — "light blacks", no colour cast.
        ink: '#0B0B0C',
        ink2: '#0F0F11',
        panel: '#151517',
        panel2: '#1C1C1F',
        // These two carry their own alpha, so Tailwind's `/n` modifier must
        // not be used on them: `border-line/60` *replaces* the 0.07 with 0.6
        // rather than scaling it, and draws a hairline nine times brighter
        // than intended. To fade something, change its background.
        line: 'rgba(255,255,255,0.07)',
        lineBright: 'rgba(255,255,255,0.16)',
        // Monochrome accent scale. Emphasis comes from luminance, never hue.
        basalt: '#F4F4F5',
        basaltDeep: '#B6B6BC',
        basaltDim: '#74747A',
        text: '#F4F4F5',
        textDim: '#9A9AA1',
        textFaint: '#5E5E64',
        // The only colour in the system, reserved for destructive actions.
        danger: '#E08368',
        dangerBg: '#1E1614',
      },
      fontFamily: {
        sans: ["'Plus Jakarta Sans Variable'", 'system-ui', 'sans-serif'],
        display: ["'Plus Jakarta Sans Variable'", 'system-ui', 'sans-serif'],
        // A real monospace, unlike Frostbyte which mapped mono back to Jakarta.
        // This is a file manager: paths, byte counts and hashes all want columns
        // that line up.
        mono: ["'Geist Mono Variable'", 'ui-monospace', 'monospace'],
      },
      letterSpacing: {
        tightest: '-0.03em',
        tighter: '-0.02em',
      },
      borderRadius: {
        sm: '8px',
        md: '12px',
        lg: '16px',
        xl: '20px',
        '2xl': '26px',
      },
      boxShadow: {
        basalt:
          '0 1px 0 0 rgba(255,255,255,0.08) inset, 0 10px 30px -12px rgba(0,0,0,0.8)',
        lift: '0 18px 50px -12px rgba(0,0,0,0.75)',
      },
      keyframes: {
        shimmer: {
          '0%': { transform: 'translateX(-120%)' },
          '100%': { transform: 'translateX(220%)' },
        },
        breathe: {
          '0%,100%': { opacity: '0.5' },
          '50%': { opacity: '0.85' },
        },
      },
      animation: {
        shimmer: 'shimmer 2.4s ease-in-out infinite',
        breathe: 'breathe 3.5s ease-in-out infinite',
      },
    },
  },
  plugins: [],
} satisfies Config
