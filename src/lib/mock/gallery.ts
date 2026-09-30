// Mock for the model details page (browser dev mode / tests).
import type { MockTable } from "./index";
import type { CoreError, GalleryItem, ModelGallery } from "../types";
import { mockSettings } from "./app";
import { catalogEntryByVersion } from "./catalog";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const err = (code: string, message: string): CoreError => ({ code, message, details: null });

const SUBJECTS = [
  "a lighthouse on a cliff at dusk, warm light, mist over the sea",
  "a cozy reading nook with plants and afternoon sun",
  "a red fox in fresh snow, soft morning light",
  "a futuristic city street in the rain, neon reflections",
  "a bowl of ramen on a wooden table, steam, shallow depth of field",
  "a paper-cut mountain landscape, layered, pastel colors",
];
const SIZES: [number, number][] = [
  [832, 1216],
  [1024, 1024],
  [1216, 832],
];

function gallery(versionId: number, content: string, modelNsfw: boolean): ModelGallery {
  const card = catalogEntryByVersion(versionId);
  const look = card?.styleBadge?.toLowerCase().replace(/\s+/g, "_") ?? "realistic";
  const items: GalleryItem[] = [];
  let hidden = 0;
  for (let i = 0; i < 9; i++) {
    const nsfw = modelNsfw && i % 4 === 3;
    if (nsfw && content === "safe") {
      hidden++;
      continue;
    }
    const [w, h] = SIZES[i % SIZES.length];
    items.push({
      index: i,
      thumbUrl: `https://image.civitai.com/mock/${versionId}${i}/${look}.jpeg`,
      fullUrl: `https://image.civitai.com/mock/${versionId}${i}/${look}.jpeg`,
      width: w,
      height: h,
      nsfw,
      generation:
        i % 5 === 4
          ? null
          : {
              prompt: SUBJECTS[i % SUBJECTS.length],
              negativePrompt: i % 2 ? "blurry, low quality, watermark" : undefined,
              steps: 20 + (i % 3) * 5,
              sampler: i % 2 ? "DPM++ 2M Karras" : "Euler a",
              cfgScale: 4 + (i % 3),
              seed: 1000 + i * 7919,
              Size: `${w}x${h}`,
            },
    });
  }
  return { items, hiddenNsfw: hidden, trainedWords: card?.type === "LORA" ? ["mockstyle"] : [], creatorNotes: { model: MOCK_NOTES, version: "<p>v2 fixes <b>hands</b> and adds a softer look.</p>" }, offline: false };
}

const MOCK_NOTES =
  '<h2>About this model</h2><p>A <strong>general-purpose</strong> model for <em>everyday scenes</em>. Tips:</p><ul><li>Use 25 steps</li><li>Keep the prompt short</li></ul>' +
  '<p>Support me: <a href="https://example.com/creator">my page</a></p><img src="https://example.com/tracker.png"><script>alert(1)</script>' + // privacy-lint: allow mock creator text; the sanitizer drops the image and script
  "<p>Long text. ".repeat(40) + "</p>";

const table: MockTable = {
  model_gallery: async (a) => {
    if (mockSettings().offline) return { items: [], hiddenNsfw: 0, trainedWords: [], offline: true } satisfies ModelGallery;
    await sleep(400);
    return gallery(Number(a.versionId), String(a.content), Boolean(a.modelNsfw));
  },
  open_external_link: async () => {},
  open_civitai_page: async (a) => {
    if (!Number(a.modelId)) throw err("invalid", "That model has no CivitAI page.");
  },
};

export default table;
