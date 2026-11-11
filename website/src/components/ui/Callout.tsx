import type { ReactNode } from "react";

const TONES = {
  info: { box: "border-primary/40 bg-primary/10", icon: "text-link", path: "M12 16v-4M12 8h.01" },
  warn: { box: "border-warn/40 bg-warn/10", icon: "text-warn", path: "M12 9v4M12 17h.01" },
} as const;

export function Callout({ tone = "info", title, children }: { tone?: keyof typeof TONES; title?: string; children: ReactNode }) {
  const t = TONES[tone];
  return (
    <div className={`flex gap-3 rounded-xl border p-4 text-sm leading-relaxed ${t.box}`} role="note">
      <svg className={`mt-0.5 shrink-0 ${t.icon}`} width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
        <circle cx="12" cy="12" r="10" />
        <path d={t.path} />
      </svg>
      <div>
        {title && <p className="font-semibold text-fg">{title}</p>}
        <div className="text-muted [&_strong]:text-fg">{children}</div>
      </div>
    </div>
  );
}
