import { links } from "../data/links";
import { VERSION } from "../data/generated";
import { Button } from "./ui/Button";
import { CodeWindow, CommandLine } from "./ui/CodeWindow";
import { HeroGraph } from "./HeroGraph";
import { GitHubIcon } from "./GitHubStars";

export function Hero() {
  return (
    <section id="top" className="relative isolate overflow-hidden">
      <HeroGraph />
      <div className="mx-auto grid max-w-6xl items-center gap-12 px-5 pb-20 pt-14 sm:px-8 lg:grid-cols-[1.05fr_1fr] lg:pb-28 lg:pt-20">
        <div className="min-w-0">
          <p className="mb-5 inline-flex items-center gap-2 rounded-full border border-line bg-surface/70 px-3 py-1 font-mono text-xs text-muted">
            <span className="size-1.5 rounded-full bg-ok" aria-hidden="true" />
            v{VERSION} · Graph Database. Open Source.
          </p>
          <h1 className="text-4xl font-semibold leading-[1.08] sm:text-5xl lg:text-6xl">
            A lightweight <span className="gradient-text-light">graph database.</span>
          </h1>
          <p className="mt-5 max-w-xl text-lg leading-relaxed text-muted">
            Cypher queries, a Rust engine and an MIT license. Embed it in your program or run it as a server in one
            command.
          </p>
          <div className="mt-8 flex flex-wrap gap-3">
            <Button href="#get-started">Get started</Button>
            <Button href={links.repo} variant="secondary">
              <GitHubIcon size={16} /> View on GitHub
            </Button>
          </div>
          <div className="mt-6 max-w-xl">
            <CommandLine id="docker-run" />
          </div>
        </div>
        <div className="min-w-0 lg:pl-4">
          <CodeWindow id="quickstart" filename="quickstart.py" className="shadow-2xl shadow-primary/10" />
          <p className="mt-3 text-center text-xs text-muted">
            Python client: <code className="inline-code">pip install mygraph-client</code>
          </p>
        </div>
      </div>
    </section>
  );
}
