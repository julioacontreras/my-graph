import { useEffect, useState } from "react";
import { MINOR } from "../data/generated";
import { links } from "../data/links";

const KEY = `mygraph-announce-${MINOR}-dismissed`;

export function AnnouncementBar() {
  const [open, setOpen] = useState(true);
  useEffect(() => {
    try {
      if (localStorage.getItem(KEY)) setOpen(false);
    } catch {
      /* storage blocked */
    }
  }, []);
  if (!open) return null;
  const dismiss = () => {
    setOpen(false);
    try { localStorage.setItem(KEY, "1"); } catch { /* storage blocked */ }
  };
  return (
    <div className="relative bg-gradient-to-r from-primary/25 via-violet/20 to-primary/25 text-sm">
      <div className="mx-auto flex min-h-10 max-w-6xl items-center justify-center px-12 py-2 text-center">
        <a href={links.changelog} className="text-fg hover:underline underline-offset-4">
          <span className="font-semibold">v{MINOR} is out:</span> weighted shortest paths, a path cache and concurrent reads{" "}
          <span aria-hidden="true">→</span>
        </a>
      </div>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss announcement"
        className="absolute right-2 top-1/2 inline-flex size-9 -translate-y-1/2 items-center justify-center rounded-md text-muted hover:text-fg"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
