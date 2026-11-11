import { StrictMode } from "react";
import { createRoot, hydrateRoot } from "react-dom/client";
import "./fonts";
import "./styles/index.css";
import { App } from "./App";

const root = document.getElementById("root")!;
const app = (
  <StrictMode>
    <App />
  </StrictMode>
);
// Production HTML is prerendered (scripts/prerender.mjs): hydrate it. In dev, render.
if (root.hasChildNodes()) hydrateRoot(root, app);
else createRoot(root).render(app);
