import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { installMocks, isTauri } from "./lib/mock";
import { blockStrayDrops, markPlatform } from "./lib/platform";
import "./index.css";

async function boot() {
  markPlatform();
  // A drop outside the image drop areas must never navigate the window away from the app.
  blockStrayDrops();
  // Outside Tauri (plain browser / screenshots) use the mock backend.
  if (!isTauri()) await installMocks();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}

void boot();
