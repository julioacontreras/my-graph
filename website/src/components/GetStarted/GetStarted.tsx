import { links } from "../../data/links";
import { Section } from "../ui/Section";
import { Callout } from "../ui/Callout";
import { CodeWindow } from "../ui/CodeWindow";
import { DockerSteps } from "./DockerSteps";
import { OtherInstalls } from "./OtherInstalls";

const DOCS = [
  { label: "Quick reference", href: links.quickRef },
  { label: "Cypher support", href: links.cypher },
  { label: "Indexes", href: links.indexes },
  { label: "Python client", href: links.python },
  { label: "Wire protocol", href: links.protocol },
  { label: "GitHub", href: links.repo },
];

export function GetStarted() {
  return (
    <Section
      id="get-started"
      eyebrow="Get started"
      title="Your first query in under a minute."
      intro={<p>Docker is the quickest path: start the server, connect with a client, done.</p>}
    >
      <DockerSteps />

      <div className="mt-20 grid [&>*]:min-w-0 gap-10 lg:grid-cols-[1.4fr_1fr]">
        <div className="reveal">
          <h3 className="mb-4 text-xl font-semibold">Other ways to install</h3>
          <OtherInstalls />
        </div>
        <div className="reveal space-y-6">
          <Callout title="Create an index on every property you use to find nodes.">
            <p>
              Indexes are <strong>not automatic</strong>. This is the most important performance tip: in v0.2 the
              100k-relationship load went from 28.1 s to under 1 s once the lookup key was indexed.
            </p>
            <div className="mt-3"><CodeWindow id="index" filename="index.cypher" /></div>
          </Callout>
          <nav aria-label="Documentation">
            <p className="mb-3 text-sm font-semibold">Documentation</p>
            <ul className="grid grid-cols-2 gap-2 text-sm">
              {DOCS.map((d) => (
                <li key={d.label}>
                  <a href={d.href} className="flex min-h-11 items-center justify-between rounded-lg border border-line px-3 text-muted transition-colors hover:border-muted hover:text-fg">
                    {d.label} <span aria-hidden="true">↗</span>
                  </a>
                </li>
              ))}
            </ul>
          </nav>
        </div>
      </div>
    </Section>
  );
}
