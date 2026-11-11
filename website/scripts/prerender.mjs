// Renders the React app to static HTML in dist/index.html, so the page works
// before JavaScript loads and search engines see the full text.
// Optional: SITE_URL=https://mygraph.dev adds absolute URLs, a canonical link and sitemap.xml.
import { readFile, writeFile, rm } from "node:fs/promises";

const dist = new URL("../dist/", import.meta.url);
const ssr = new URL("../dist-ssr/entry-server.js", import.meta.url);
const { render, VERSION } = await import(ssr.href);

const site = process.env.SITE_URL?.replace(/\/$/, "");
const jsonLd = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: "MyGraph",
  description: "An MIT-licensed property-graph database written in Rust and queried with Cypher.",
  applicationCategory: "DeveloperApplication",
  operatingSystem: "Linux, macOS (Docker: amd64, arm64)",
  softwareVersion: VERSION,
  license: "https://opensource.org/licenses/MIT",
  codeRepository: "https://github.com/julioacontreras/my-graph",
  downloadUrl: ["https://hub.docker.com/r/julioacontreras/mygraph", "https://pypi.org/project/mygraph-client/"],
  offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
  ...(site ? { url: site } : {}),
};

let head = `<script type="application/ld+json">${JSON.stringify(jsonLd)}</script>`;
let html = await readFile(new URL("index.html", dist), "utf8");
if (site) {
  head += `\n    <link rel="canonical" href="${site}/" />\n    <meta property="og:url" content="${site}/" />`;
  // Vite may have prefixed the base path already (e.g. /my-graph/og.png).
  html = html.replace(/content="[^"]*\/og\.png"/g, `content="${site}/og.png"`);
  await writeFile(
    new URL("sitemap.xml", dist),
    `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>${site}/</loc></url></urlset>\n`,
  );
  await writeFile(new URL("robots.txt", dist), `User-agent: *\nAllow: /\nSitemap: ${site}/sitemap.xml\n`);
}

html = html.replace("<!--head-->", head).replace("<!--app-->", render());
await writeFile(new URL("index.html", dist), html);
await rm(new URL("../dist-ssr/", import.meta.url), { recursive: true, force: true });
console.log(`prerender: dist/index.html (${Math.round(html.length / 1024)} KB)${site ? `, ${site}` : ""}`);
