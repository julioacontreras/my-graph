import { StrictMode } from "react";
import { renderToString } from "react-dom/server";
import { App } from "./App";
import { VERSION } from "./data/generated";

export { VERSION };

export function render() {
  return renderToString(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
