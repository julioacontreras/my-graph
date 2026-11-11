// llms-full.txt = the README "Usage", "Reference" and "Status" sections, as written for agents.
import { readFile, writeFile } from "node:fs/promises";

const root = new URL("../../", import.meta.url);
const site = new URL("../", import.meta.url);
const readme = await readFile(new URL("README.md", root), "utf8");

const start = readme.indexOf("\n## Usage");
const end = readme.indexOf("\n## Development");
if (start < 0 || end < 0) throw new Error("README sections not found (## Usage / ## Development)");

const body = readme
  .slice(start, end)
  .replaceAll("](PROTOCOL.md)", "](https://github.com/julioacontreras/my-graph/blob/main/PROTOCOL.md)")
  .replaceAll("](python/README.md)", "](https://github.com/julioacontreras/my-graph/blob/main/python/README.md)")
  .replaceAll("](python/)", "](https://github.com/julioacontreras/my-graph/tree/main/python)")
  .replaceAll("](docker-compose.yml)", "](https://github.com/julioacontreras/my-graph/blob/main/docker-compose.yml)")
  .replaceAll("](#", "](https://github.com/julioacontreras/my-graph#");

const out = `# MyGraph — full quick reference

> Generated from the README of https://github.com/julioacontreras/my-graph.
> MyGraph is an MIT-licensed property-graph database written in Rust and queried with Cypher.

${body.trim()}
`;
await writeFile(new URL("public/llms-full.txt", site), out);
console.log(`build-llms: ${out.length} chars`);
