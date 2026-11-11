import { useState } from "react";
import { links } from "../data/links";
import { Logo } from "./ui/Logo";
import { GitHubIcon, formatStars, useStars } from "./GitHubStars";

const NAV = [
  { href: links.quickRef, label: "Docs" },
  { href: "#benchmarks", label: "Benchmarks" },
  { href: links.changelog, label: "Changelog" },
];

export function Navbar() {
  const stars = useStars();
  const [open, setOpen] = useState(false);
  return (
    <header className="sticky top-0 z-40 border-b border-line/60 bg-bg/80 backdrop-blur-md">
      <nav className="mx-auto flex h-16 max-w-6xl items-center justify-between px-5 sm:px-8" aria-label="Main">
        <a href="#top" aria-label="MyGraph home" className="rounded-md">
          <Logo />
        </a>
        <div className="hidden items-center gap-1 md:flex">
          {NAV.map((n) => (
            <a key={n.label} href={n.href} className="rounded-md px-3 py-2 text-sm text-muted transition-colors hover:text-fg">
              {n.label}
            </a>
          ))}
          <a
            href={links.repo}
            className="ml-1 inline-flex min-h-9 items-center gap-2 rounded-md border border-line px-3 text-sm text-muted transition-colors hover:border-muted hover:text-fg"
          >
            <GitHubIcon size={16} />
            GitHub
            {stars !== null && stars > 0 && (
              <span className="rounded bg-elevated px-1.5 text-xs text-fg" aria-label={`${stars} stars`}>
                ★ {formatStars(stars)}
              </span>
            )}
          </a>
          <a href="#get-started" className="ml-2 inline-flex min-h-9 items-center rounded-md bg-primary px-4 text-sm font-semibold text-white hover:bg-primary-hover">
            Get started
          </a>
        </div>
        <button
          type="button"
          className="inline-flex size-11 items-center justify-center rounded-md text-muted hover:text-fg md:hidden"
          aria-expanded={open}
          aria-controls="mobile-menu"
          aria-label="Menu"
          onClick={() => setOpen((o) => !o)}
        >
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
            {open ? <path d="M18 6 6 18M6 6l12 12" /> : <path d="M4 7h16M4 12h16M4 17h16" />}
          </svg>
        </button>
      </nav>
      {open && (
        <div id="mobile-menu" className="border-t border-line bg-bg px-5 pb-4 md:hidden">
          {[...NAV, { href: links.repo, label: "GitHub" }].map((n) => (
            <a key={n.label} href={n.href} onClick={() => setOpen(false)} className="block py-3 text-muted hover:text-fg">
              {n.label}
            </a>
          ))}
          <a href="#get-started" onClick={() => setOpen(false)} className="mt-2 flex min-h-11 items-center justify-center rounded-md bg-primary font-semibold text-white">
            Get started
          </a>
        </div>
      )}
    </header>
  );
}
