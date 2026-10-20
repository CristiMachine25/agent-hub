/** The app mark: a relay baton made of quota segments, handed on. Same drawing as app-icon.svg. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden>
      <defs>
        <clipPath id="logo-baton">
          <rect x="15" y="24" width="42" height="16" rx="8" />
        </clipPath>
      </defs>
      <rect width="64" height="64" rx="14" fill="#1A1918" />
      <g transform="rotate(-38 32 32)">
        <path d="M3 27.5 H10 M5 36.5 H10" stroke="#9AA5FF" strokeWidth="3.2" strokeLinecap="round" />
        <g clipPath="url(#logo-baton)">
          <rect x="15" y="24" width="10" height="16" fill="#E2BE5C" />
          <rect x="26.5" y="24" width="9.5" height="16" fill="#B7C565" />
          <rect x="37.5" y="24" width="9.5" height="16" fill="#86C877" />
          <rect x="48.5" y="24" width="8.5" height="16" fill="#5EC98C" />
        </g>
      </g>
    </svg>
  )
}
