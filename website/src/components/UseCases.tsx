import { CodeWindow } from "./ui/CodeWindow";
import { Section } from "./ui/Section";

const FITS = [
  {
    title: "Knowledge graphs and agent memory",
    body: "Entities and relations an agent can query with Cypher through the CLI (--json) or Python.",
  },
  { title: "Embedded graphs in Rust applications", body: "A graph store inside your binary, with no separate server." },
  { title: "Products shipped to customers", body: "MIT lets you bundle the database with what you sell." },
];

export function UseCases() {
  return (
    <Section id="use-cases" eyebrow="Use cases" title="Built for connected data.">
      <div className="reveal grid [&>*]:min-w-0 items-start gap-8 rounded-2xl border border-primary/40 bg-gradient-to-br from-primary/10 to-violet/5 p-7 lg:grid-cols-[1fr_1.15fr]">
        <div>
          <span className="rounded-full bg-ok/15 px-2.5 py-1 text-xs font-semibold text-ok">Proven · measured</span>
          <h3 className="mt-4 text-2xl font-semibold">Code and dependency analysis</h3>
          <p className="mt-3 leading-relaxed text-muted">
            Programs, jobs, tables and fields as a graph: the impact of changing a field, data lineage, dead code, call
            cycles, and the shortest path between components. This is the workload of the benchmark.
          </p>
        </div>
        <CodeWindow id="cypher-paths" filename="impact.cypher" />
      </div>
      <p className="mb-4 mt-10 text-sm font-semibold text-muted">Good fit: supported by current features, not yet measured in production</p>
      <ul className="grid gap-4 md:grid-cols-3">
        {FITS.map((f) => (
          <li key={f.title} className="reveal rounded-2xl border border-line p-6">
            <h3 className="text-lg font-semibold">{f.title}</h3>
            <p className="mt-2 text-sm leading-relaxed text-muted">{f.body}</p>
          </li>
        ))}
      </ul>
    </Section>
  );
}
