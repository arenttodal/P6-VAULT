import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import { setBackend } from "./api/backend";
import { tauriBackend } from "./api/tauri";
import "./styles/app.css";
import { startImport } from "./hooks/useEvents";
import { useApp } from "./stores/app";

setBackend(tauriBackend);

// Automation hooks for the WebDriver end-to-end test (scripts/e2e.mjs). They only reach
// the same typed commands the UI uses; native file dialogs cannot be automated.
(window as unknown as Record<string, unknown>).__P6_TEST__ = { startImport, state: () => useApp.getState() };

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
