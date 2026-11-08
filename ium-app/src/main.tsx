import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initTheme } from "./theme";
import "./theme.css";

// The theme lands before the first paint (#86): the light base is the
// no-attribute default, dark rides [data-theme="dark"] — a dark-preferred
// boot never flashes light. The watcher keeps a `system` pref live.
initTheme();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
