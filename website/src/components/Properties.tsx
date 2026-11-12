import { links } from "../data/links";
import { Section } from "./ui/Section";
import { MINOR } from "../data/generated";

const PROPS = [
  {
    title: "Cypher you already know.",
    body: (
      <>
        <code className="inline-code">MATCH</code>, <code className="inline-code">OPTIONAL MATCH</code>,{" "}
        <code className="inline-code">WITH</code>, <code className="inline-code">UNWIND</code>, aggregations,
        variable-length paths and <code className="inline-code">shortestPath</code>, by hops or by a weight
        property. Index syntax from both Neo4j and Memgraph.
      </>
    ),
    href: links.cypher,
  },
  {
    title: "Embedded or server, same engine.",
    body: (
      <>
        <code className="inline-code">Database::open(path)</code> in Rust, or{" "}
        <code className="inline-code">mygraph-server</code> over TCP with a Python client and a CLI.
      </>
    ),
    href: links.usage,
  },
  {
    title: "Durable by default.",
    body: "Write-ahead log with CRC32 and fsync on commit, snapshots, crash recovery, and transactions with rollback.",
    href: links.status,
  },
  {
    title: "Light on memory.",
    body: "Rust, no garbage collector, no heap to size. 280 MB with 100k relationships in our v0.2 benchmark.",
    href: "#benchmarks",
  },
  {
    title: "MIT licensed.",
    body: "Ship it inside your product. No paid edition, no license fees.",
    href: links.license,
  },
];

const SECONDARY = [
  "Multiple databases and schemas",
  "Property indexes and a rule-based planner",
  "--json output for scripts and agents",
  "Docker image for amd64 and arm64",
  "Errors with line and column",
];

export function Properties() {
  return (
    <Section id="features" eyebrow="Features" title="Five properties of the engine.">
      <ol className="grid gap-px overflow-hidden rounded-2xl border border-line bg-line md:grid-cols-2 lg:grid-cols-3">
        {PROPS.map((p, i) => (
          <li key={p.title} className={`reveal flex flex-col bg-bg p-7 ${i === 0 ? "lg:row-span-1" : ""}`}>
            <span className="font-mono text-sm text-link">{String(i + 1).padStart(2, "0")}</span>
            <h3 className="mt-3 text-xl font-semibold">{p.title}</h3>
            <p className="mt-2 flex-1 text-sm leading-relaxed text-muted">{p.body}</p>
            <a href={p.href} className="link mt-5 text-sm">
              Learn more <span aria-hidden="true">→</span>
              <span className="sr-only"> about: {p.title}</span>
            </a>
          </li>
        ))}
        <li className="reveal flex flex-col justify-center bg-surface p-7">
          <p className="mb-3 text-sm font-semibold">Also in v{MINOR}</p>
          <ul className="space-y-2 text-sm text-muted">
            {SECONDARY.map((s) => (
              <li key={s} className="flex gap-2">
                <svg className="mt-0.5 shrink-0 text-link" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg>
                {s}
              </li>
            ))}
          </ul>
        </li>
      </ol>
    </Section>
  );
}
