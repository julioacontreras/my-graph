import type { ReactNode } from "react";
import data from "../../data/benchmarks.json";

const MY = data.engines.length - 1;
// As in the report: one decimal below 100 ms, whole numbers above.
const fmt = (n: number) => (n < 100 ? n.toFixed(1) : Math.round(n).toLocaleString("en-US"));
const fmtInt = (n: number) => n.toLocaleString("en-US");
const isNumeric = (v: string) => /^[\d,.]+\s*(ms|s|MB)?$/.test(v);

type TableProps = { caption: string; head?: ReactNode; children: ReactNode; firstCol?: string; textCols?: boolean; plainNames?: boolean };

function Table({ caption, head, children, firstCol = "", textCols, plainNames }: TableProps) {
  return (
    <div className="overflow-x-auto rounded-xl border border-line">
      <table className="w-full min-w-[720px] border-collapse text-left text-sm">
        <caption className="sr-only">{caption}</caption>
        <thead className="bg-surface text-xs text-muted">
          <tr>
            <th scope="col" className="px-4 py-3 font-medium">{firstCol}</th>
            {data.engines.map((e, i) => (
              <th key={e.id} scope="col" className={`px-3 py-3 font-medium ${textCols ? "text-left" : "text-right"} ${i === MY ? "bg-primary/15 text-fg" : ""}`}>
                {plainNames ? e.name.replace(/ v[\d.]+$/, "") : e.name}
              </th>
            ))}
            {head}
          </tr>
        </thead>
        <tbody className="divide-y divide-line">{children}</tbody>
      </table>
    </div>
  );
}

function Cell({ children, best, my, left }: { children: ReactNode; best?: boolean; my?: boolean; left?: boolean }) {
  return (
    <td className={`px-3 py-2.5 ${left ? "text-left" : "text-right font-mono text-[13px]"} ${my ? "bg-primary/10" : ""} ${best ? "font-bold text-fg" : my ? "text-fg" : "text-muted"}`}>
      {best ? <strong>{children}</strong> : children}
    </td>
  );
}

const minIndex = (vals: (number | null)[]) => {
  let best = -1;
  vals.forEach((v, i) => { if (v !== null && (best < 0 || v < (vals[best] as number))) best = i; });
  return best;
};

export function SummaryTable() {
  return (
    <Table caption={`Summary. ${data.summary.caption}`} textCols>
      {data.summary.rows.map((r) => (
        <tr key={r.label}>
          <th scope="row" className="px-4 py-2.5 font-medium">{r.label}</th>
          {r.values.map((v, i) => (
            <Cell key={i} best={r.best.includes(i)} my={i === MY} left>{isNumeric(v) ? <span className="font-mono text-[13px]">{v}</span> : v}</Cell>
          ))}
        </tr>
      ))}
    </Table>
  );
}

export function SearchTable() {
  return (
    <Table
      caption="Searches at ×500, median of 10 warm runs, in milliseconds"
      firstCol="Query (ms)"
      head={<th scope="col" className="px-4 py-3 text-right font-medium">Rows</th>}
    >
      {data.searches.rows.map((r) => {
        const best = minIndex(r.values);
        return (
          <tr key={r.id}>
            <th scope="row" className="px-4 py-2.5 font-medium">
              <span className="mr-2 font-mono text-xs text-link">{r.id}</span>{r.label}
            </th>
            {r.values.map((v, i) => <Cell key={i} best={i === best} my={i === MY}>{fmt(v)}</Cell>)}
            <td className="px-4 py-2.5 text-right font-mono text-[13px] text-muted">{fmtInt(r.rowCount)}</td>
          </tr>
        );
      })}
    </Table>
  );
}

export function UpdateTable() {
  return (
    <Table caption="Updates at ×500, in milliseconds" firstCol="Operation (ms)">
      {data.updates.rows.map((r) => {
        const best = minIndex(r.values);
        return (
          <tr key={r.id}>
            <th scope="row" className="px-4 py-2.5 font-medium">
              <span className="mr-2 font-mono text-xs text-link">{r.id}</span>{r.label}
            </th>
            {r.values.map((v, i) => (
              <Cell key={i} best={i === best} my={i === MY}>
                {v === null ? <span className="font-sans text-danger">crashed (OOM)</span> : fmt(v)}
              </Cell>
            ))}
          </tr>
        );
      })}
    </Table>
  );
}

export function LicenseTable() {
  return (
    <Table caption={`Licenses, checked ${data.licenses.checked}`} textCols plainNames>
      {data.licenses.rows.map((r) => (
        <tr key={r.label}>
          <th scope="row" className="px-4 py-2.5 font-medium">{r.label}</th>
          {r.values.map((v, i) => <Cell key={i} my={i === MY} left>{v}</Cell>)}
        </tr>
      ))}
    </Table>
  );
}
