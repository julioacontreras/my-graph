import data from "../../data/benchmarks.json";

const MY = data.engines.length - 1; // MyGraph is always the last engine
const fmt = (n: number) => n.toLocaleString("en-US");

type BarRow = { label: string; value: number; highlight?: boolean };

/** Horizontal bars, sorted, with direct labels. MyGraph in brand blue, others neutral gray. */
export function HBarChart({ title, rows, unit, note }: { title: string; rows: BarRow[]; unit: string; note?: string }) {
  const max = Math.max(...rows.map((r) => r.value));
  const sorted = [...rows].sort((a, b) => a.value - b.value);
  return (
    <figure className="rounded-2xl border border-line bg-surface/50 p-6">
      <figcaption className="mb-5">
        <p className="font-semibold">{title}</p>
        <p className="text-xs text-muted">Lower is better · {unit}</p>
      </figcaption>
      <ul className="space-y-3">
        {sorted.map((r) => (
          <li key={r.label} className="grid grid-cols-[6.5rem_1fr] items-center gap-3 text-sm">
            <span className={r.highlight ? "font-semibold text-fg" : "text-muted"}>{r.label}</span>
            <span className="flex items-center gap-2">
              <span
                className={`h-5 rounded-r ${r.highlight ? "bg-gradient-to-r from-primary to-violet" : "bg-[#3b4660]"}`}
                style={{ width: `${Math.max((r.value / max) * 82, 1.5)}%` }}
                aria-hidden="true"
              />
              <span className={`whitespace-nowrap font-mono text-xs ${r.highlight ? "text-fg" : "text-muted"}`}>
                {fmt(r.value)} {unit}
              </span>
            </span>
          </li>
        ))}
      </ul>
      {note && <p className="mt-5 text-xs leading-relaxed text-muted">{note}</p>}
    </figure>
  );
}

export function MemoryChart() {
  return (
    <HBarChart
      title="Memory after load (×500)"
      unit="MB"
      rows={data.engines.map((e, i) => ({ label: e.name, value: data.memoryMB[i], highlight: i === MY }))}
      note="Neo4j and ArcadeDB memory is mostly pre-allocated to the JVM (Neo4j 2 GB heap + 2 GB page cache; ArcadeDB 3 GB heap + 1 GB pages), so it barely grows with the graph."
    />
  );
}

export function SearchSumChart() {
  return (
    <HBarChart
      title="Sum of the 13 searches (×500, p50)"
      unit="ms"
      rows={data.engines.map((e, i) => ({ label: e.name, value: data.searches.sums[i], highlight: i === MY }))}
    />
  );
}

/** Grouped bars: load throughput per scale. Higher is better. */
export function LoadChart() {
  const max = Math.max(...data.load.rows.flatMap((r) => r.values));
  return (
    <figure className="rounded-2xl border border-line bg-surface/50 p-6">
      <figcaption className="mb-5 flex flex-wrap items-end justify-between gap-2">
        <div>
          <p className="font-semibold">Load throughput</p>
          <p className="text-xs text-muted">Higher is better · {data.load.unit}, batches of 1,000</p>
        </div>
      </figcaption>
      <div className="space-y-6">
        {data.load.rows.map((row) => (
          <div key={row.scale}>
            <p className="mb-2 text-sm">
              <span className="font-mono font-semibold">{row.scale}</span>{" "}
              <span className="text-xs text-muted">
                {fmt(row.nodes)} nodes · {fmt(row.relationships)} relationships
              </span>
            </p>
            <ul className="space-y-1.5">
              {data.engines.map((e, i) => (
                <li key={e.id} className="grid grid-cols-[6.5rem_1fr] items-center gap-3 text-xs">
                  <span className={i === MY ? "font-semibold text-fg" : "text-muted"}>{e.name}</span>
                  <span className="flex items-center gap-2">
                    <span
                      className={`h-3 rounded-r ${i === MY ? "bg-gradient-to-r from-primary to-violet" : "bg-[#3b4660]"}`}
                      style={{ width: `${Math.max((row.values[i] / max) * 80, 1)}%` }}
                      aria-hidden="true"
                    />
                    <span className={`font-mono ${i === MY ? "text-fg" : "text-muted"}`}>{fmt(row.values[i])}</span>
                  </span>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      <p className="mt-5 text-xs leading-relaxed text-muted">
        v0.1 had no property indexes, so every batch of relationships scanned all nodes. With an index on the lookup key
        (since v0.2), the ×500 load takes 5.2 s instead of 28.1 s. Keeping the indexes up to date costs some throughput at
        ×500.
      </p>
    </figure>
  );
}
