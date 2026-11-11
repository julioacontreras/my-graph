import { links } from "../data/links";
import { Button } from "./ui/Button";
import { GitHubIcon } from "./GitHubStars";

export function FinalCta() {
  return (
    <section className="relative isolate overflow-hidden border-t border-line/60 py-24 sm:py-32" aria-labelledby="cta-title">
      <div className="pointer-events-none absolute left-1/2 top-1/2 -z-10 h-[360px] w-[720px] -translate-x-1/2 -translate-y-1/2 rounded-full bg-gradient-to-r from-primary/30 to-violet/30 blur-[120px]" aria-hidden="true" />
      <div className="reveal mx-auto max-w-3xl px-5 text-center">
        <h2 id="cta-title" className="text-4xl font-semibold sm:text-5xl">
          Run your first <span className="gradient-text-light">graph query.</span>
        </h2>
        <div className="mt-9 flex flex-wrap justify-center gap-3">
          <Button href="#get-started">Get started</Button>
          <Button href={links.repo} variant="secondary"><GitHubIcon size={16} /> Star on GitHub</Button>
        </div>
        <p className="mt-6 text-sm text-muted">MIT licensed. No account. Runs on your laptop.</p>
      </div>
    </section>
  );
}
