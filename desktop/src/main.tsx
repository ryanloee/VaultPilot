import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/globals.css";
import { applyTheme, savedTheme } from "./lib/theme";

// Apply the persisted theme before first paint so the dark palette is active
// immediately on startup (previously it was only applied on button click).
applyTheme(savedTheme() ?? "system");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
