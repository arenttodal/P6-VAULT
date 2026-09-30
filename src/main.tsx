import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./app/App";
import { setBackend } from "./api/backend";
import { tauriBackend } from "./api/tauri";
import "./styles/app.css";

setBackend(tauriBackend);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
