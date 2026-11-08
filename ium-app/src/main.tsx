import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Launch } from "./components/Launch";
import { applyPlatformMaterial, initTheme } from "./theme";
import { api } from "./api";
import "./theme.css";

// The theme lands before the first paint (#86): the light base is the
// no-attribute default, dark rides [data-theme="dark"] — a dark-preferred
// boot never flashes light. The watcher keeps a `system` pref live.
initTheme();

// The materials verdict (#92) lands pre-paint — a Mica boot never flashes
// opaque. Windows 11 only (the engine's OS-build gate); every other
// platform keeps the opaque window, and a plain-browser dev run (no Tauri)
// falls back to opaque exactly like a refused verdict.
api
  .platformMaterial()
  .then((mica) => applyPlatformMaterial(mica))
  .catch(() => applyPlatformMaterial(false));

// The launch window (#87) rides the same entry point under #/launch — one
// bundle, two surfaces: the splash renders the 96 px identity moment, the
// main window renders the working chrome. No router, no second build.
const isLaunch = window.location.hash === "#/launch";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isLaunch ? <Launch /> : <App />}</React.StrictMode>,
);
