import { Section } from "./ui/Section";

const typical = [
  "Runs on the JVM",
  "2–4 GB of heap and page cache reserved at start-up",
  "Always a separate server process",
  "GPL or source-available (BSL) free edition",
];
const ours = [
  "One Rust binary, no garbage collector",
  "230 MB after loading 100k relationships*",
  "A library in your process, or a TCP server",
  "MIT licensed: ship it inside your product",
];

export function WhyMyGraph() {
  return (
    <Section
      id="why"
      eyebrow="Why MyGraph"
      title="Graph databases don't have to be heavy."
      intro={
        <p>
          Most graph databases run on the JVM, reserve gigabytes of heap at start-up, and come with GPL or
          source-available licenses. MyGraph is one Rust binary, or a library inside your own process.
        </p>
      }
    >
      <div className="reveal grid gap-5 md:grid-cols-2">
        <figure className="rounded-2xl border border-line bg-surface/50 p-7">
          <figcaption className="mb-6 flex items-center justify-between">
            <span className="font-display text-lg font-semibold text-muted">Typical graph server</span>
            <span className="rounded-md border border-line px-2 py-0.5 font-mono text-xs text-muted">JVM</span>
          </figcaption>
          <div className="mb-6 flex h-28 items-end gap-2" aria-hidden="true">
            <div className="flex h-full flex-1 flex-col justify-end rounded-lg border border-dashed border-line p-2">
              <div className="h-[85%] rounded-md bg-[#3b4660]/70 p-2 font-mono text-[10px] text-muted">heap + page cache</div>
            </div>
            <div className="flex h-full w-20 items-center justify-center rounded-lg border border-line font-mono text-[10px] text-muted">your app</div>
          </div>
          <ul className="space-y-3 text-sm text-muted">
            {typical.map((t) => (
              <li key={t} className="flex gap-3"><span className="mt-2 size-1.5 shrink-0 rounded-full bg-[#3b4660]" />{t}</li>
            ))}
          </ul>
        </figure>
        <figure className="relative rounded-2xl border border-primary/40 bg-gradient-to-br from-primary/10 to-violet/10 p-7">
          <figcaption className="mb-6 flex items-center justify-between">
            <span className="font-display text-lg font-semibold">MyGraph</span>
            <span className="rounded-md border border-primary/50 px-2 py-0.5 font-mono text-xs text-link">Rust</span>
          </figcaption>
          <div className="mb-6 flex h-28 items-end gap-2" aria-hidden="true">
            <div className="flex h-full flex-1 flex-col justify-end rounded-lg border border-line p-2">
              <div className="flex h-[9%] min-h-5 items-center rounded-md bg-gradient-to-r from-primary to-violet px-2 font-mono text-[10px] text-white">graph</div>
            </div>
            <div className="flex h-full w-20 flex-col items-center justify-center gap-1 rounded-lg border border-primary/50 font-mono text-[10px] text-fg">
              your app<span className="rounded bg-primary/30 px-1">+ mygraph</span>
            </div>
          </div>
          <ul className="space-y-3 text-sm">
            {ours.map((t) => (
              <li key={t} className="flex gap-3"><span className="mt-2 size-1.5 shrink-0 rounded-full bg-link" />{t}</li>
            ))}
          </ul>
        </figure>
      </div>
      <p className="mt-5 text-xs text-muted">
        * Memory after load at ×500 (50,677 nodes, 100,525 relationships), measured on MyGraph v0.1.0. Neo4j used 2,793 MB
        and ArcadeDB 3,410 MB in the same test, mostly memory pre-allocated to the JVM.{" "}
        <a href="#benchmarks" className="link">See the benchmark</a>.
      </p>
    </Section>
  );
}
