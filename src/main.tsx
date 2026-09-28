import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { installMocks, isTauri } from "./lib/mock";
import "./index.css";

async function boot() {
  // Outside Tauri (plain browser / screenshots) use the mock backend.
  if (!isTauri()) await installMocks();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void boot();
