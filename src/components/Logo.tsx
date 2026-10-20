/** The app mark: a track switched onto the branch with quota left. Same drawing as app-icon.svg. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden>
      <defs>
        <linearGradient id="logo-tile" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#2A2826" />
          <stop offset="1" stopColor="#161514" />
        </linearGradient>
        <linearGradient id="logo-route" x1="0" y1="1" x2="1" y2="0">
          <stop offset="0" stopColor="#9AA5FF" />
          <stop offset="1" stopColor="#5EC98C" />
        </linearGradient>
      </defs>
      <rect width="64" height="64" rx="14" fill="url(#logo-tile)" />
      <g fill="none" stroke="#4A4640" strokeWidth="4.5" strokeLinecap="round">
        <path d="M24 44 H44" />
        <path d="M24 44 C33 44 34 32 44 32" />
      </g>
      <circle cx="50" cy="44" r="3.5" fill="#F06868" />
      <circle cx="50" cy="32" r="3.5" fill="#E2BE5C" />
      <path d="M12 44 H24 C35 44 34 20 44 20" fill="none" stroke="url(#logo-route)" strokeWidth="5.5" strokeLinecap="round" />
      <circle cx="50" cy="20" r="4.5" fill="#5EC98C" />
    </svg>
  )
}
