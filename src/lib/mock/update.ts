// Mock handlers for Settings → Updates. See ./index.ts.
//
// URL flag: ?update=none | installer (default) | portable | appImage | manual | private (GitHub 404)
import type { MockTable } from "./index";
import type { CoreError, UpdateCheck, UpdateInstallMode } from "../types";
import { mockSettings } from "./app";
import { startMockDownload } from "./models";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const offlineError: CoreError = { code: "offline", message: "Offline mode is on. Turn it off in Settings to check for updates.", details: null };

function mode(): UpdateInstallMode | "none" {
  const v = new URLSearchParams(typeof location !== "undefined" ? location.search : "").get("update");
  return v === "none" || v === "portable" || v === "appImage" || v === "manual" ? v : "installer";
}

let githubToken = false;

const table: MockTable = {
  check_for_updates: async (): Promise<UpdateCheck> => {
    await sleep(700);
    if (mockSettings().offline) throw offlineError;
    if (new URLSearchParams(location.search).get("update") === "private" && !githubToken)
      throw { code: "updates_unavailable", message: "Pinhole can't see its releases on GitHub, because the project isn't public yet. Add a GitHub token below, or download new versions from the release page.", details: "HTTP 404" };
    const m = mode();
    if (m === "none") return { currentVersion: "0.1.0", update: null };
    return {
      currentVersion: "0.1.0",
      update: { version: "0.2.0", publishedAt: "2026-09-28T12:00:00Z", installMode: m, sizeBytes: m === "manual" ? null : 14 * 1024 * 1024 },
    };
  },
  install_update: ({ version }) =>
    new Promise<void>((_resolve, reject) => {
      if (mockSettings().offline) return reject(offlineError);
      // Success never resolves: the real app quits and restarts.
      startMockDownload(`Pinhole ${String(version)}`, [{ name: `Pinhole ${String(version)}`, bytes: 14 * 1024 * 1024 }], {
        kind: "appUpdate",
        durationMs: 4000,
        onFail: reject,
      });
    }),
  open_release_page: async () => undefined,
  has_github_token: async () => githubToken,
  set_github_token: async () => {
    githubToken = true;
  },
  clear_github_token: async () => {
    githubToken = false;
  },
};

export default table;
