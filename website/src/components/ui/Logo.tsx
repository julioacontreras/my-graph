/** The graph mark from branding/logo.png, redrawn as SVG: 6 nodes, blue → violet. */
export function LogoMark({ size = 28, glow = false }: { size?: number; glow?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true" style={glow ? { filter: "drop-shadow(0 0 6px rgba(0,108,252,.6))" } : undefined}>
      <defs>
        <linearGradient id="mg-edge" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#2f86ff" />
          <stop offset="1" stopColor="#8b2cf5" />
        </linearGradient>
      </defs>
      <g stroke="url(#mg-edge)" strokeWidth="2.2" strokeLinecap="round" opacity=".9">
        <line x1="9" y1="23" x2="41" y2="7" />
        <line x1="9" y1="23" x2="31" y2="33" />
        <line x1="41" y1="7" x2="31" y2="33" />
        <line x1="31" y1="33" x2="52" y2="40" />
        <line x1="31" y1="33" x2="17" y2="47" />
        <line x1="31" y1="33" x2="40" y2="56" />
        <line x1="9" y1="23" x2="17" y2="47" />
        <line x1="17" y1="47" x2="40" y2="56" />
      </g>
      <circle cx="41" cy="7" r="5.5" fill="#1a8bff" />
      <circle cx="9" cy="23" r="6" fill="#1170ff" />
      <circle cx="31" cy="33" r="7.5" fill="#0a63f0" />
      <circle cx="52" cy="40" r="4" fill="#8b2cf5" />
      <circle cx="17" cy="47" r="6" fill="#8b2cf5" />
      <circle cx="40" cy="56" r="5.5" fill="#1a6dff" />
    </svg>
  );
}

/** Light wordmark for dark backgrounds: "My" near-white, "Graph" in the brand gradient. */
export function Logo() {
  return (
    <span className="flex items-center gap-2.5">
      <LogoMark />
      <span className="font-display text-xl font-semibold tracking-tight">
        <span className="text-fg">My</span>
        <span className="gradient-text-light">Graph</span>
      </span>
    </span>
  );
}
