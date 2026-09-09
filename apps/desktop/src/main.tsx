import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { desktopAvailable } from "./api";
import { ProviderSettingsLauncher } from "./ProviderSettings";
import "./style.css";
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
    <ProviderSettingsLauncher available={desktopAvailable} />
  </React.StrictMode>,
);
