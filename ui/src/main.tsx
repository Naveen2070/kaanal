// Entry point: take the token off the address bar, then mount <App /> on #root.

import { NuqsAdapter } from "nuqs/adapters/react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./main.css";
import { initTheme } from "./look/theme/theme";
import { applyStartMode } from "./data/store/mode";
import { captureToken } from "./token";

captureToken(window);
applyStartMode(window);
// boot.js already did this before the first paint; this covers a dev page without it
initTheme();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <NuqsAdapter>
      <App />
    </NuqsAdapter>
  </StrictMode>,
);
