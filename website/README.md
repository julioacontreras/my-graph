# MyGraph website

Static landing page for MyGraph. Vite + React + Tailwind, prerendered to plain HTML.

```bash
npm install
npm run dev       # http://localhost:5173
npm run build     # → dist/ (static files, deploy as is)
npm run preview   # serve dist/
SITE_URL=https://mygraph.dev npm run build   # adds canonical URL, sitemap.xml and robots.txt
BASE_PATH=/my-graph/ npm run build          # serve from a sub-path (GitHub Pages project site)
```

## Where the content comes from

| What | Source |
|---|---|
| Version | Root `Cargo.toml` (`scripts/build-data.mjs` → `src/data/generated.ts`) |
| GitHub stars | GitHub API at build time, refreshed in the browser |
| Benchmark numbers | `src/data/benchmarks.json` (from the v0.1 benchmark report). Never type numbers into components |
| Code snippets | `src/data/snippets/*`, highlighted by Shiki at build time (`scripts/highlight-snippets.mjs`) |
| `/llms-full.txt` | README "Usage" + "Reference" + "Status" (`scripts/build-llms.mjs`) |
| `/docker-compose.yml` | Copied from the repo root |

`scripts/prerender.mjs` renders the app to `dist/index.html`. The browser then hydrates it,
so the page works and is indexable before JavaScript loads.

## Deploy

`.github/workflows/pages.yml` builds the site and publishes it to GitHub Pages on every push to `main`
that touches `website/`, the root `README.md`, `Cargo.toml` or `docker-compose.yml`. It sets `BASE_PATH`
and `SITE_URL` from the Pages configuration, so a custom domain needs no code change.
