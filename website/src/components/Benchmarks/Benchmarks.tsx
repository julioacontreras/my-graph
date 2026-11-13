import data from "../../data/benchmarks.json";
import { Section } from "../ui/Section";
import { Callout } from "../ui/Callout";
import { LoadChart, MemoryChart, SearchSumChart } from "./Charts";
import { LicenseTable, SearchTable, SummaryTable, UpdateTable, VolumeTable } from "./Tables";

function Sub({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="reveal mt-16">
      <h3 className="mb-5 text-2xl font-semibold">{title}</h3>
      {children}
    </div>
  );
}

export function Benchmarks() {
  return (
    <Section
      id="benchmarks"
      eyebrow="Benchmarks"
      title="Honest numbers."
      intro={
        <p>
          MyGraph measured against Neo4j, Memgraph, Dgraph and ArcadeDB on the same machine, with the same limits and
          the same queries. Including where we lose.
        </p>
      }
    >
      <Callout title={`Measured on MyGraph v${data.mygraphVersion}.`}>
        Since v0.2, the 13 searches went from 769 ms to 557 ms in total, and memory after the ×500 load from 280 MB to
        186 MB. The ×500 load takes 5.2 s (28.1 s in v0.1). The test queries still use the v0.1 workarounds, so the
        traversals do not yet use variable-length paths.
      </Callout>

      <ul className="mt-8 grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {data.highlights.map((h) => (
          <li key={h.value} className="reveal rounded-2xl border border-line bg-gradient-to-b from-surface to-bg p-6">
            <p className="gradient-text-light font-display text-3xl font-semibold">{h.value}</p>
            <p className="mt-2 text-sm leading-relaxed text-muted">{h.text}</p>
          </li>
        ))}
      </ul>

      <Sub title="Summary">
        <p className="mb-4 text-sm text-muted">{data.summary.caption}. Bold = best in the row.</p>
        <SummaryTable />
      </Sub>

      <div className="reveal mt-10 grid gap-5 lg:grid-cols-2">
        <MemoryChart />
        <SearchSumChart />
      </div>

      <Sub title="Load">
        <LoadChart />
      </Sub>

      <Sub title="Searches at ×500">
        <p className="mb-4 text-sm text-muted">Median of 10 warm runs, in ms. Bold = fastest.</p>
        <SearchTable />
        <p className="mt-3 text-xs leading-relaxed text-muted">
          For MyGraph, Q1, Q5, Q6, Q7 and Q12 still walk the graph from the application, and Q8 and Q11 are split into
          several queries, as in v0.1. MyGraph supports variable-length paths and{" "}
          <code className="inline-code">shortestPath</code> since v0.2, but the test queries have not been rewritten
          yet, so most of the v0.3 search improvements do not show here. Property indexes are created by hand, the same
          ones as in Neo4j.
        </p>
      </Sub>

      <Sub title="Updates at ×500">
        <p className="mb-4 text-sm text-muted">In ms. Bold = fastest.</p>
        <UpdateTable />
        <p className="mt-3 text-xs leading-relaxed text-muted">
          U3, U3b and U4 run once (single timing).
        </p>
      </Sub>

      <Sub title="Volume test: a tree of 1 million nodes">
        <p className="mb-4 text-sm text-muted">
          {data.volume.caption}. Loaded into an empty database in batches of 10,000. Times in ms (p50, warm). Bold =
          best in the row.
        </p>
        <VolumeTable />
        <p className="mt-3 text-xs leading-relaxed text-muted">
          MyGraph completes every test and is the fastest at the short traversals (T3, T4), but this is where it loses
          most: the load takes 210.5 s and memory reaches 5,949 MB, close to the 6 GB limit, and the count per level
          (T5) is the slowest. Dgraph ran out of memory after T4 and restarted.
        </p>
      </Sub>

      <Sub title="Licenses">
        <LicenseTable />
        <p className="mt-3 text-xs leading-relaxed text-muted">
          Checked {data.licenses.checked}. Sources:{" "}
          {data.licenses.sources.map((s, i) => (
            <span key={s.name}>
              <a className="link" href={s.url}>{s.name}</a>
              {i < data.licenses.sources.length - 1 ? " · " : "."}
            </span>
          ))}
        </p>
      </Sub>

      <details className="reveal group mt-16 rounded-2xl border border-line bg-surface/40 p-6 open:bg-surface/60">
        <summary className="flex min-h-11 cursor-pointer list-none items-center justify-between text-lg font-semibold">
          Methodology and limits
          <span className="text-muted transition-transform group-open:rotate-45" aria-hidden="true">+</span>
        </summary>
        <div className="mt-4 grid gap-8 text-sm leading-relaxed text-muted md:grid-cols-2">
          <ul className="list-disc space-y-2 pl-5">
            <li><strong className="text-fg">Dataset:</strong> a hand-written base system (131 nodes, 226 relationships) of legacy programs, jobs, tables and their translation layers, cloned and cross-linked to three scales, so long traversals cross subsystems. It includes dangling relationships, a call cycle and dead code on purpose.</li>
            <li><strong className="text-fg">Tests:</strong> load in batches of 1,000; 13 searches; 7 updates (with their reverse where it applies).</li>
            <li><strong className="text-fg">Volume test:</strong> apart from the scales, a full tree of 1,082,401 nodes (4 levels, 32 children per node; 1,048,576 leaves) is loaded, traversed at several depths and modified. The first run of each search is checked against the result computed from the shape of the tree.</li>
            <li><strong className="text-fg">Runs:</strong> each search once cold and 10 times warm; the median (p50) is reported.</li>
            <li><strong className="text-fg">Correctness:</strong> all five engines return the same number of rows in every test and scale.</li>
            <li><strong className="text-fg">Environment:</strong> a laptop, Podman on macOS. Each engine measured alone (the others stopped), limited to 4 CPU and 6 GB, on a clean volume. Neo4j and ArcadeDB memory set by hand (Neo4j 2 GB heap + 2 GB page cache; ArcadeDB 3 GB heap + 1 GB pages).</li>
            <li>
              <strong className="text-fg">Versions:</strong>{" "}
              {data.versions.map((v, i) => (
                <span key={v}><code className="inline-code">{v}</code>{i < data.versions.length - 1 ? ", " : "."}</span>
              ))}
            </li>
          </ul>
          <div>
            <p className="mb-2 font-semibold text-fg">Limits of this benchmark</p>
            <ul className="list-disc space-y-2 pl-5">
              <li>A laptop, not a server: it compares engines with each other; it does not size production.</li>
              <li>One connection, no concurrency. High availability and failover were not measured.</li>
              <li>Synthetic data (a cloned base system), with the shape and volume of a real snapshot.</li>
              <li>Dgraph and ArcadeDB ran on a single node; distribution does not matter at this volume.</li>
              <li>With all engines running at once, the 12 GB VM ran out of memory. That is why each engine is measured alone.</li>
            </ul>
            <p className="mt-4">The harness, dataset generator and raw results will be published in the repository. Harness coming soon.</p>
          </div>
        </div>
      </details>

      <p className="reveal mt-12 text-center font-display text-xl text-muted">
        Small enough to embed. Durable enough to trust. <span className="text-fg">Measured in the open.</span>
      </p>
    </Section>
  );
}
