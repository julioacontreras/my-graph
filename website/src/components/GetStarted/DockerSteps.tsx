import type { ReactNode } from "react";
import { links } from "../../data/links";
import { MINOR, VERSION } from "../../data/generated";
import { CodeWindow, CommandLine } from "../ui/CodeWindow";
import { CopyButton } from "../ui/CopyButton";
import { Tabs } from "../ui/Tabs";
import { Callout } from "../ui/Callout";

function Step({ n, title, children }: { n: number; title: string; children: ReactNode }) {
  return (
    <li className="reveal relative grid gap-5 pb-14 pl-14 last:pb-0">
      <span className="absolute left-0 top-0 flex size-9 items-center justify-center rounded-full border border-primary/60 bg-primary/15 font-mono text-sm text-fg">
        {n}
      </span>
      <span className="absolute bottom-0 left-[17px] top-11 w-px bg-line" aria-hidden="true" />
      <h3 className="pt-1 text-xl font-semibold">{title}</h3>
      {children}
    </li>
  );
}

const MANAGE: { cmd: string; what: ReactNode; danger?: boolean }[] = [
  { cmd: "docker compose ps", what: "Check that it is running" },
  { cmd: "docker compose logs -f mygraph", what: "Follow the logs" },
  { cmd: "docker compose down", what: "Stop it. The data stays in the volume" },
  { cmd: "docker compose pull && docker compose up -d", what: "Update to the latest image" },
  { cmd: "docker compose down -v", what: <>Stop it and <strong>delete all data</strong></>, danger: true },
];

export function DockerSteps() {
  return (
    <ol className="max-w-4xl">
      <Step n={1} title="Start the server">
        <Tabs
          label="How to start the server"
          tabs={[
            { id: "docker", label: "Docker", content: <CommandLine id="docker-run" /> },
            {
              id: "compose",
              label: "Docker Compose",
              content: (
                <div className="space-y-3">
                  <CodeWindow id="docker-compose" filename="docker-compose.yml" />
                  <CommandLine id="compose-up" />
                  <p className="text-sm text-muted">
                    Or <a className="link" href={`${import.meta.env.BASE_URL}docker-compose.yml`} download>download docker-compose.yml</a> (the
                    file in the{" "}
                    <a className="link" href={links.composeFile}>repo</a>).
                  </p>
                </div>
              ),
            },
          ]}
        />
        <ul className="grid gap-x-8 gap-y-2 text-sm text-muted sm:grid-cols-2">
          <li><span aria-hidden="true">🖥️</span> Image for <strong className="text-fg">amd64 and arm64</strong> (Apple Silicon and Linux servers).</li>
          <li>
            <span aria-hidden="true">📦</span> On <a className="link" href={links.dockerHub}>Docker Hub</a> (<code className="inline-code">julioacontreras/mygraph</code>) and{" "}
            <a className="link" href={links.ghcr}>GHCR</a> (<code className="inline-code">ghcr.io/julioacontreras/mygraph</code>).
          </li>
          <li>
            <span aria-hidden="true">🏷️</span> Tags: <code className="inline-code">latest</code>, <code className="inline-code">{VERSION}</code>,{" "}
            <code className="inline-code">{MINOR}</code>. Pin a version in production.
          </li>
          <li><span aria-hidden="true">🔌</span> Port <strong className="text-fg">6660</strong>. Data in the volume <code className="inline-code">mygraph-data</code>, mounted at <code className="inline-code">/data</code>.</li>
        </ul>
      </Step>

      <Step n={2} title="Connect and run a query">
        <Callout tone="warn" title="The image contains only the server.">
          It has no shell and no CLI, so <code className="inline-code">docker exec</code> /{" "}
          <code className="inline-code">docker compose exec</code> cannot run queries. Connect from your machine with a
          client.
        </Callout>
        <CodeWindow id="connect" filename="terminal" />
        <p className="text-sm text-muted">
          Both CLIs connect to <code className="inline-code">localhost:6660</code> by default. Use{" "}
          <code className="inline-code">--host</code> / <code className="inline-code">--port</code> for another address.
        </p>
      </Step>

      <Step n={3} title="Manage the container">
        <div className="overflow-x-auto rounded-xl border border-line">
          <table className="w-full text-left text-sm">
            <caption className="sr-only">Docker Compose commands</caption>
            <thead className="bg-surface text-muted">
              <tr>
                <th scope="col" className="px-4 py-3 font-medium">Command</th>
                <th scope="col" className="px-4 py-3 font-medium">What it does</th>
                <th scope="col" className="w-0 px-2 py-3"><span className="sr-only">Copy</span></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {MANAGE.map((m) => (
                <tr key={m.cmd} className={m.danger ? "bg-danger/5" : ""}>
                  <td className="px-4 py-2.5">
                    <code className={`whitespace-nowrap font-mono text-[13px] ${m.danger ? "text-danger" : "text-fg"}`}>{m.cmd}</code>
                  </td>
                  <td className={`px-4 py-2.5 ${m.danger ? "text-danger" : "text-muted"}`}>{m.what}</td>
                  <td className="px-2 py-1.5"><CopyButton text={m.cmd} label="" /></td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <div className="grid gap-3 md:grid-cols-2">
          <Callout tone="warn" title="No authentication.">
            <code className="inline-code">AUTH</code> is accepted but not checked, and there is no TLS. Do not publish
            port 6660 to the internet: bind it to localhost (<code className="inline-code">-p 127.0.0.1:6660:6660</code>)
            or keep it on a private network.
          </Callout>
          <Callout title="Upgrading.">
            0.4 reads data from every earlier version. Data written by 0.4 cannot be read by 0.3 or earlier, so a
            downgrade needs the backup. 0.1 cannot read data written by 0.2 or later. Back up the volume before
            upgrading.{" "}
            <a className="link" href={links.changelog}>Changelog</a>
          </Callout>
        </div>
      </Step>
    </ol>
  );
}
