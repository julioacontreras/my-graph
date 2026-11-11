import { links } from "../data/links";
import { CodeWindow } from "./ui/CodeWindow";
import { Section } from "./ui/Section";

const ITEMS = [
  {
    title: "--json output",
    body: (
      <>
        Both CLIs print JSON with one stable shape for nodes, relationships and paths.{" "}
        <a className="link" href={links.resultFormat}>Result format</a>
      </>
    ),
  },
  {
    title: "llms.txt",
    body: (
      <>
        <a className="link" href={links.llms}>/llms.txt</a> and <a className="link" href={links.llmsFull}>/llms-full.txt</a>,
        generated from the README quick reference.
      </>
    ),
  },
  {
    title: "Quick reference",
    body: (
      <>
        Written for agents: what is supported, how to connect, and what fails.{" "}
        <a className="link" href={links.quickRef}>Read it</a>
      </>
    ),
  },
];

export function ForAgents() {
  return (
    <Section id="agents" eyebrow="For AI agents" title="Readable by humans and agents.">
      <div className="grid [&>*]:min-w-0 items-start gap-8 lg:grid-cols-[1fr_1.2fr]">
        <ul className="reveal space-y-4">
          {ITEMS.map((it) => (
            <li key={it.title} className="rounded-xl border border-line p-5">
              <p className="font-mono text-sm font-semibold text-fg">{it.title}</p>
              <p className="mt-1.5 text-sm leading-relaxed text-muted">{it.body}</p>
            </li>
          ))}
        </ul>
        <div className="reveal space-y-3">
          <CodeWindow id="agent-json" filename="terminal" />
          <p className="text-xs text-muted">
            The CLIs cannot pass parameters: put literal values in the query, or use the Python or Rust library when
            values come from user input.
          </p>
        </div>
      </div>
    </Section>
  );
}
