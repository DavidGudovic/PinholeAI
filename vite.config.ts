/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed dev port and no screen clearing.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2022",
    sourcemap: false,
    // Everything is bundled locally; no CDN assets (privacy rule 4).
    assetsInlineLimit: 0,
  },
  test: {
    environment: "jsdom",
    // One jsdom per worker; each test file still gets its own VM context.
    pool: "vmThreads",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
