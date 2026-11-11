import { links } from "../data/links";
import { VERSION } from "../data/generated";
import { Logo } from "./ui/Logo";

const COLS: { title: string; items: [string, string][] }[] = [
  {
    title: "Product",
    items: [["Get started", "#get-started"], ["Quick reference", links.quickRef], ["Benchmarks", "#benchmarks"], ["Changelog", links.changelog], ["Protocol", links.protocol]],
  },
  { title: "Packages", items: [["Docker Hub", links.dockerHub], ["GHCR", links.ghcr], ["PyPI", links.pypi]] },
  { title: "Community", items: [["GitHub", links.repo], ["Issues", links.issues], ["Pull requests", links.pulls]] },
  { title: "Machine-readable", items: [["llms.txt", links.llms], ["llms-full.txt", links.llmsFull]] },
];

export function Footer() {
  return (
    <footer className="border-t border-line/60 py-14 text-sm">
      <div className="mx-auto grid max-w-6xl gap-10 px-5 sm:px-8 md:grid-cols-[1.4fr_repeat(4,1fr)]">
        <div>
          <Logo />
          <p className="mt-3 text-muted">Graph Database. Open Source.</p>
          <p className="mt-4 font-mono text-xs text-muted">v{VERSION} · <a className="hover:text-fg" href={links.license}>MIT License</a></p>
        </div>
        {COLS.map((c) => (
          <nav key={c.title} aria-label={c.title}>
            <p className="mb-3 font-semibold">{c.title}</p>
            <ul className="space-y-2">
              {c.items.map(([label, href]) => (
                <li key={label}><a className="text-muted transition-colors hover:text-fg" href={href}>{label}</a></li>
              ))}
            </ul>
          </nav>
        ))}
      </div>
    </footer>
  );
}
