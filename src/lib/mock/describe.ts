// Mock handlers for the describe area. See ./index.ts.
// URL flag: ?captioner → the describer is already available (screenshots).
import { invoke } from "@tauri-apps/api/core";
import type { MockTable } from "./index";
import { startMockDownload } from "./models";
import type { CaptionerStatus, CoreError, InstalledModel, RecommendedPick } from "../types";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });
const MB = 1024 * 1024;

let installed = typeof location !== "undefined" && new URLSearchParams(location.search).has("captioner");
let warm = false;

async function status(): Promise<CaptionerStatus> {
  const [models, picks] = await Promise.all([
    invoke<InstalledModel[]>("list_models").catch(() => [] as InstalledModel[]),
    invoke<RecommendedPick[]>("get_recommended").catch(() => [] as RecommendedPick[]),
  ]);
  const reuse = models.some((m) => /qwen_image_edit|qwen_image/.test(m.familyId ?? "") && !m.missingComponents.length);
  const viaRecommended = picks.find((p) => p.role === "describe")?.installed ?? false;
  const available = installed || reuse || viaRecommended;
  return { available, source: reuse ? "reuse" : available ? "default" : null, downloadBytes: available ? 0 : (1900 + 850) * MB, running: warm };
}

/** Rough colour words from the image so the canned text isn't always identical. */
async function mood(bytes: ArrayBuffer): Promise<{ tone: string; light: string }> {
  try {
    const bmp = await createImageBitmap(new Blob([bytes]));
    const c = document.createElement("canvas");
    c.width = 16;
    c.height = 16;
    const g = c.getContext("2d")!;
    g.drawImage(bmp, 0, 0, 16, 16);
    const d = g.getImageData(0, 0, 16, 16).data;
    let r = 0,
      gr = 0,
      b = 0;
    for (let i = 0; i < d.length; i += 4) {
      r += d[i];
      gr += d[i + 1];
      b += d[i + 2];
    }
    const n = d.length / 4;
    r /= n;
    gr /= n;
    b /= n;
    const lum = 0.2126 * r + 0.7152 * gr + 0.0722 * b;
    const tone = r > b + 20 ? "warm golden" : b > r + 20 ? "cool blue" : gr > r && gr > b ? "fresh green" : "soft neutral";
    return { tone, light: lum > 150 ? "bright, airy light" : lum < 80 ? "low, moody light" : "gentle, even light" };
  } catch {
    return { tone: "soft neutral", light: "gentle, even light" };
  }
}

const table: MockTable = {
  captioner_status: () => status(),
  install_captioner: async () => {
    await sleep(200);
    const groupId = startMockDownload(
      "Describe model",
      [
        { name: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf", bytes: 1900 * MB },
        { name: "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf", bytes: 850 * MB },
      ],
      { kind: "captioner", durationMs: 5000, onDone: () => (installed = true) },
    );
    return { groupId };
  },
  describe_image: async (a) => {
    const st = await status();
    if (!st.available) throw err("not_found", "Describing needs the small vision model first. Click “Get the describer”.");
    const bytes = await invoke<ArrayBuffer>("get_image", { id: a.imageId });
    await sleep(warm ? 1400 : 3200);
    warm = true;
    const { tone, light } = await mood(bytes);
    if (a.style === "tags") {
      return `no humans, scenery, outdoors, sky, sun, mountains, hills, landscape, ${tone.replace(" ", "_")}_tones, ${light.includes("moody") ? "dusk" : "daylight"}, depth_of_field, film_grain, highly_detailed`;
    }
    return `A wide landscape at golden hour with layered hills fading into haze, a glowing sun low in the sky and ${tone} tones throughout. ${light[0].toUpperCase()}${light.slice(1)} with soft atmospheric depth, fine film grain, calm and quiet mood, shot from a slightly elevated viewpoint.`;
  },
};

export default table;
