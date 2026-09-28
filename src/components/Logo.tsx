/** Pinhole mark (mirrors assets/icon.svg). Inline SVG — no external assets. */
export function Logo({ className = "h-6 w-6" }: { className?: string }) {
  return (
    <svg viewBox="0 0 1024 1024" className={className} aria-hidden>
      <defs>
        <radialGradient id="pinhole-logo-g" cx="50%" cy="50%" r="50%">
          <stop offset="0" stopColor="#fff7d6" />
          <stop offset="0.35" stopColor="#ffd166" />
          <stop offset="1" stopColor="#ef8a17" />
        </radialGradient>
      </defs>
      <rect x="64" y="64" width="896" height="896" rx="200" fill="#16181d" />
      <circle cx="512" cy="512" r="300" fill="none" stroke="#2b2f38" strokeWidth="56" />
      <circle cx="512" cy="512" r="210" fill="#0b0c0f" />
      <circle cx="512" cy="512" r="62" fill="url(#pinhole-logo-g)" />
      <path d="M512 450 L760 250" stroke="#ffd166" strokeOpacity="0.35" strokeWidth="18" strokeLinecap="round" />
    </svg>
  );
}
