// Drop a picture Pinhole saved onto Create to reuse how it was made. Pictures carry their settings
// only when "Settings (no prompt)" was on at save time; the prompt is never in there, so the prompt
// box stays as it is. The picture itself is not kept: only the settings chunk is read, in Rust.
import * as api from "../../lib/api";
import type { ParsedGeneration } from "../../lib/paste/parse";
import { SD_SAMPLERS, SD_SCHEDULERS, type PasteSkip } from "../../lib/paste/map";
import type { Actions } from "../../lib/state/actions";
import { createModels } from "../../lib/state/model";
import type { Store } from "../../lib/state/store";
import type { PictureSettings } from "../../lib/types";
import { applyParsed } from "./pasteApply";

/** Same limit as adding a picture (Rust refuses bigger ones anyway). */
const MAX_BYTES = 64 * 1024 * 1024;

export interface ReuseOutcome {
  /** The picture carried settings. */
  found: boolean;
  /** Model name written in the picture. */
  madeWith: string | null;
  /** That model is installed here and was selected. */
  modelSelected: boolean;
  /** The model that will be used now. */
  modelName: string | null;
  applied: string[];
  skipped: PasteSkip[];
}

const NOTHING: ReuseOutcome = { found: false, madeWith: null, modelSelected: false, modelName: null, applied: [], skipped: [] };

/** The saved settings as the paste mapper's input (sampler and scheduler are set afterwards: they are already the engine's names). */
export function parsedFromPicture(s: PictureSettings): ParsedGeneration {
  return {
    prompt: "",
    negative: null,
    steps: s.steps ?? null,
    cfg: s.cfg ?? null,
    guidance: s.guidance ?? null,
    sampler: null,
    scheduler: null,
    seed: s.seed ?? null,
    width: s.width ?? null,
    height: s.height ?? null,
    clipSkip: null,
    denoise: null,
    hires: null,
    modelName: s.model ?? null,
    modelHash: null,
    loraTags: [],
    hypernetworks: [],
    resources: [],
    extra: {},
    hasSettings: true,
  };
}

/** Apply the settings of a dropped picture to Create. `found: false` when it carries none. */
export async function reuseSettingsFrom(file: Blob, store: Store, actions: Actions): Promise<ReuseOutcome> {
  if (file.size > MAX_BYTES) return NOTHING;
  const s = await api.readPictureSettings(new Uint8Array(await file.arrayBuffer()));
  if (!s) return NOTHING;

  const usable = createModels(store.getState().models);
  // The exact model first, then one with the same name (older pictures don't name the id).
  const same = usable.find((m) => s.modelId && m.id === s.modelId) ?? usable.find((m) => s.model && m.friendlyName === s.model) ?? null;
  const o = await applyParsed(parsedFromPicture(s), null, null, store, actions, { setPrompt: false, preferModelId: same?.id ?? null, keepLoras: true });

  const fineTune: { sampler?: string; scheduler?: string } = {};
  if (s.sampler && SD_SAMPLERS.some((x) => x.id === s.sampler)) fineTune.sampler = s.sampler;
  if (s.scheduler && SD_SCHEDULERS.some((x) => x.id === s.scheduler)) fineTune.scheduler = s.scheduler;
  if (fineTune.sampler || fineTune.scheduler) store.dispatch({ type: "setFineTune", patch: fineTune });

  return { found: true, madeWith: s.model ?? null, modelSelected: !!same, modelName: o.modelName, applied: o.applied, skipped: o.skipped };
}
