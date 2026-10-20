/** The app mark: three quota bars and a hop from the emptiest to the fullest. Same drawing as app-icon.svg. */
export function Logo({ size = 24 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden>
      <rect width="64" height="64" rx="14" fill="#0E141E" />
      <rect x="11" y="39" width="11" height="14" rx="2.5" fill="#F25C6E" />
      <rect x="26.5" y="29" width="11" height="24" rx="2.5" fill="#E6BE45" />
      <rect x="42" y="19" width="11" height="34" rx="2.5" fill="#3DD39B" />
      <path d="M16.5 33 C 19 10, 40 4, 47.5 13" fill="none" stroke="#A193FF" strokeWidth="4.5" strokeLinecap="round" />
      <path d="M41.5 12.5 L 48.5 14 L 49 6.8" fill="none" stroke="#A193FF" strokeWidth="4.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
