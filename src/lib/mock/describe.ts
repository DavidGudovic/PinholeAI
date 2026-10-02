// Mock handlers for the describe area. See ./index.ts.
// URL flag: ?captioner → the describer is already available (screenshots).
import { invoke } from "@tauri-apps/api/core";
import type { MockTable } from "./index";
import { startMockDownload } from "./models";
import { requireLicence } from "./licences";
import type { CaptionerStatus, CoreError, HelperModel, InstalledModel, RecommendedPick } from "../types";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string, details: string | null = null): CoreError => ({ code, message, details });
const MB = 1024 * 1024;

let installed = typeof location !== "undefined" && new URLSearchParams(location.search).has("captioner");
let warm = false;
// ?helper7 → the 7B helper is installed too (screenshots).
let installed7 = typeof location !== "undefined" && new URLSearchParams(location.search).has("helper7");

async function status(): Promise<CaptionerStatus> {
  const [models, picks] = await Promise.all([
    invoke<InstalledModel[]>("list_models").catch(() => [] as InstalledModel[]),
    invoke<RecommendedPick[]>("get_recommended").catch(() => [] as RecommendedPick[]),
  ]);
  const reuse = models.some((m) => /qwen_image_edit|qwen_image/.test(m.familyId ?? "") && !m.missingComponents.length);
  const viaRecommended = picks.find((p) => p.role === "describe")?.installed ?? false;
  const available = installed || installed7 || reuse || viaRecommended;
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

/** Canned "Improve my prompt": the idea plus a few fitting details (tags or sentences). */
function improved(idea: string, tags: boolean): string {
  const t = idea.trim().replace(/[.,\s]+$/, "");
  return tags
    ? `${t}, detailed, soft natural light, depth of field, rich colours, balanced composition, sharp focus, high quality`
    : `${t}. The scene is lit by soft, warm natural light with gentle depth of field, a balanced composition and rich, calm colours, rendered in fine detail.`;
}

const table: MockTable = {
  captioner_status: () => status(),
  list_helper_models: async (): Promise<HelperModel[]> => {
    const st = await status();
    const three = installed || (st.available && st.source === "default");
    return [
      { id: "describe", title: "Qwen2.5-VL 3B", note: "Small and quick. Enough for Describe and short prompt ideas.", sizeBytes: 2775 * MB, downloadBytes: three ? 0 : 2775 * MB, installed: three, removable: installed, fit: "fits", needsSafeOff: false },
      { id: "qwen25_vl_7b", title: "Qwen2.5-VL 7B", note: "Writes fuller, more careful prompts and descriptions. Large download; shared with Qwen Image Edit.", sizeBytes: 8952 * MB, downloadBytes: installed7 ? 0 : 8952 * MB, installed: installed7, removable: installed7, fit: "tight", needsSafeOff: false },
    ];
  },
  install_captioner: async (a) => {
    await sleep(200);
    const seven = a.helperId === "qwen25_vl_7b";
    // Like Rust: only the default helper has a licence, asked only when its files download.
    if ((!a.helperId || a.helperId === "describe") && !installed) requireLicence("describe");
    const groupId = startMockDownload(
      seven ? "Qwen2.5-VL 7B" : "Describe model",
      seven
        ? [
            { name: "Qwen2.5-VL-7B-Instruct-Q8_0.gguf", bytes: 8099 * MB },
            { name: "Qwen2.5-VL-7B-Instruct.mmproj-Q8_0.gguf", bytes: 853 * MB },
          ]
        : [
            { name: "Qwen2.5-VL-3B-Instruct-Q4_K_M.gguf", bytes: 1900 * MB },
            { name: "mmproj-Qwen2.5-VL-3B-Instruct-Q8_0.gguf", bytes: 850 * MB },
          ],
      { kind: "captioner", durationMs: 5000, onDone: () => (seven ? (installed7 = true) : (installed = true)) },
    );
    return { groupId };
  },
  improve_prompt: async (a) => {
    const st = await status();
    if (!st.available) throw err("not_found", "The helper model isn't installed yet.");
    await sleep(warm ? 900 : 2500);
    warm = true;
    const idea = String(a.prompt ?? "").trim();
    if (!idea) throw err("invalid", "Type a few words about your picture first.");
    if (a.target === "edit") return { text: `${idea.replace(/[.\s]+$/, "")}. Soft natural colours. Keep the composition and the lighting unchanged.`, note: null };
    const fam = String(a.familyId ?? "");
    return { text: improved(idea, /sd15|sdxl|pony|illustrious/.test(fam)), note: null };
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
