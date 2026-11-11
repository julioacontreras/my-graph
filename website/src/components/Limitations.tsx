import { links } from "../data/links";
import { Section } from "./ui/Section";

const ROWS: [string, string][] = [
  ["Graphs that fit in RAM", "Graphs larger than memory (no page-based storage)"],
  ["One writer at a time", "Many concurrent writers (no MVCC)"],
  ["Trusted networks, local or embedded use", "Authentication or TLS (AUTH is accepted but not checked)"],
  ["Cypher reads, writes and paths", "MERGE, UNION, constraints, procedures, regex"],
  ["Equality lookups by property", "Range or composite indexes"],
];

export function Limitations() {
  return (
    <Section
      id="limits"
      eyebrow="Is MyGraph right for you?"
      title="Know what you're getting."
      intro={<p>MyGraph is young. Here is where it fits today, and what it does not do yet.</p>}
    >
      <div className="reveal overflow-hidden rounded-2xl border border-line">
        <table className="w-full text-left text-sm">
          <caption className="sr-only">When to use MyGraph and what is not supported yet</caption>
          <thead>
            <tr className="text-base">
              <th scope="col" className="w-1/2 bg-ok/10 px-6 py-4 font-display font-semibold text-ok">Use MyGraph when</th>
              <th scope="col" className="w-1/2 bg-warn/10 px-6 py-4 font-display font-semibold text-warn">Not yet, if you need</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-line">
            {ROWS.map(([yes, no]) => (
              <tr key={yes}>
                <td className="px-6 py-3.5">
                  <span className="flex gap-3"><span className="text-ok" aria-hidden="true">✓</span>{yes}</span>
                </td>
                <td className="px-6 py-3.5 text-muted">
                  <span className="flex gap-3"><span className="text-warn" aria-hidden="true">–</span>{no}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-4 text-sm text-muted">
        The full list is in the <a className="link" href={links.status}>README status table</a>.
      </p>
    </Section>
  );
}
