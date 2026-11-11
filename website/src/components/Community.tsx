import type { ReactNode } from "react";
import { links } from "../data/links";
import { Section } from "./ui/Section";
import { GitHubIcon, formatStars, useStars } from "./GitHubStars";

function Card({ href, title, body, icon }: { href: string; title: string; body: ReactNode; icon: ReactNode }) {
  return (
    <li>
      <a href={href} className="reveal group flex h-full flex-col rounded-2xl border border-line p-6 transition-colors hover:border-primary/60 hover:bg-surface/60">
        <span className="text-link" aria-hidden="true">{icon}</span>
        <span className="mt-4 font-display text-lg font-semibold">{title}</span>
        <span className="mt-1.5 flex-1 text-sm leading-relaxed text-muted">{body}</span>
        <span className="mt-4 text-sm text-link" aria-hidden="true">→</span>
      </a>
    </li>
  );
}

const icon = (d: string) => (
  <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d={d} /></svg>
);

export function Community() {
  const stars = useStars();
  return (
    <Section id="community" eyebrow="Open source" title="Built in the open.">
      <ul className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        <Card
          href={links.repo}
          title="Star the repo"
          body={stars ? `${formatStars(stars)} stars so far. Help others find it.` : "Help others find it."}
          icon={<GitHubIcon size={22} />}
        />
        <Card href={links.issues} title="Report issues" body="Found a bug or a missing feature? Open an issue." icon={icon("M12 8v4M12 16h.01M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0Z")} />
        <Card href={links.pulls} title="Contribute code" body="Pull requests are welcome. Start small: tests, docs, a missing function." icon={icon("M6 3v12M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM18 9a9 9 0 0 1-9 9")} />
        <Card href={links.changelog} title="Read the changelog" body="What changed in each release, with upgrade notes." icon={icon("M4 4h16v16H4zM8 8h8M8 12h8M8 16h5")} />
      </ul>
      <div className="reveal mt-8 flex flex-wrap items-center gap-x-6 gap-y-3 text-sm text-muted">
        <a href={links.license} className="inline-flex items-center gap-2 rounded-full border border-line px-3 py-1 hover:text-fg">
          <span className="size-2 rounded-full bg-ok" aria-hidden="true" /> License: MIT
        </a>
        <span>Packages:</span>
        <a className="link" href={links.dockerHub}>Docker Hub</a>
        <a className="link" href={links.ghcr}>GHCR</a>
        <a className="link" href={links.pypi}>PyPI</a>
      </div>
    </Section>
  );
}
