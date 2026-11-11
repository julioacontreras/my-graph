import { links } from "../data/links";
import { Section } from "./ui/Section";

const DIAGRAM = ` Python client ─┐            ┌─ mygraph-cli (shell / query)
 mygraph-client ┤  TCP 6660  │
                └─> mygraph-server ──┐
                     (protocol v1)   │
 Your Rust app ──────────────────────┤
                                     v
                                  mygraph  (embedded API)
                     ┌───────────────┼────────────────┐
              mygraph-cypher    mygraph-core    mygraph-storage
          lexer, parser, planner  graph, values,   WAL, snapshots,
          executor                catalog, indexes recovery, lock`;

const PARTS = [
  { name: "Cypher pipeline", body: "Lexer → parser → semantic checks → rule-based planner → executor." },
  { name: "In-memory graph", body: "Nodes and relationships with label and property indexes." },
  { name: "Persistence", body: "WAL + snapshot checkpoints, crash recovery, torn-write detection, exclusive directory lock." },
  { name: "Wire protocol v1", body: "Documented in PROTOCOL.md. One session per connection.", href: links.protocol },
];

export function Architecture() {
  return (
    <Section
      id="architecture"
      eyebrow="Architecture"
      title="One engine, seven small crates."
      intro={<p>The same engine runs inside your Rust program or behind a TCP server. Clients talk to it with a small, documented protocol.</p>}
    >
      <div className="grid [&>*]:min-w-0 items-start gap-8 lg:grid-cols-[1.35fr_1fr]">
        <figure className="reveal overflow-hidden rounded-2xl border border-line bg-surface">
          <pre
            className="overflow-x-auto p-6 font-mono text-[12px] leading-5 text-[#c9d4e5] sm:text-[13px]"
            role="img"
            aria-label="Architecture: the Python client, mygraph-client and mygraph-cli connect over TCP port 6660 to mygraph-server, which uses the embedded mygraph API. A Rust app can use the mygraph API directly. mygraph is built on mygraph-cypher, mygraph-core and mygraph-storage."
          >
            {DIAGRAM}
          </pre>
          <figcaption className="border-t border-line px-6 py-3 text-xs text-muted">
            From the README. <a className="link" href={links.workspace}>Workspace and crates</a>
          </figcaption>
        </figure>
        <div className="reveal space-y-4">
          {PARTS.map((p) => (
            <div key={p.name} className="rounded-xl border border-line p-4">
              <p className="font-semibold">{p.href ? <a className="link" href={p.href}>{p.name}</a> : p.name}</p>
              <p className="mt-1 text-sm text-muted">{p.body}</p>
            </div>
          ))}
          <p className="text-sm leading-relaxed text-muted">
            <strong className="text-fg">Good to know:</strong> the whole graph lives in RAM, and writes are serialised.
            An open transaction blocks other connections, so keep transactions short.{" "}
            <a className="link" href={links.designDecisions}>Design decisions</a>
          </p>
        </div>
      </div>
    </Section>
  );
}
