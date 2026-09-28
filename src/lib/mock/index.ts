// Browser-only mock backend so the UI can run in plain Vite (`npm run dev`
// opened in a browser, Playwright screenshots, vitest) without Tauri.
// Activated automatically when not running inside Tauri.
//
// Each area registers handlers in its own file (owners in parentheses):
//   ./app.ts (frontend B), ./models.ts (frontend B), ./catalog.ts (frontend B),
//   ./generate.ts (frontend A), ./library.ts (frontend A), ./describe.ts (frontend A)
// A handler receives the invoke args object and returns the result (or throws a CoreError).

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

export type MockHandler = (args: Record<string, unknown>) => unknown | Promise<unknown>;
export type MockTable = Record<string, MockHandler>;

/** Emit a fake backend event (download-progress, generation-progress, …). */
export const mockEmit = (event: string, payload: unknown) => void emit(event, payload);

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window && !(window as any).__PINHOLE_MOCK__;
}

export async function installMocks(): Promise<void> {
  (window as any).__PINHOLE_MOCK__ = true;
  const tables = await Promise.all([
    import("./app").then((m) => m.default as MockTable),
    import("./models").then((m) => m.default as MockTable),
    import("./catalog").then((m) => m.default as MockTable),
    import("./generate").then((m) => m.default as MockTable),
    import("./library").then((m) => m.default as MockTable),
    import("./describe").then((m) => m.default as MockTable),
  ]);
  const table: MockTable = Object.assign({}, ...tables);
  mockIPC(
    async (cmd, args) => {
      const h = table[cmd];
      if (!h) throw { code: "internal", message: `mock: no handler for ${cmd}`, details: null };
      return h((args ?? {}) as Record<string, unknown>);
    },
    { shouldMockEvents: true },
  );
}
