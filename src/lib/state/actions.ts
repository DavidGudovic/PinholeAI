// Async app actions (IPC + state updates). Components call these; errors are
// thrown as CoreError for the caller to show with <ErrorNotice>.
//
// PRIVACY: prompt text passes through here into IPC only. Never log it.

import * as api from "../api";
import type { CoreError, EngineStatus, FamilyUi, GenerateRequest, InstalledModel, LoraUse, ResultImage } from "../types";
import {
  DEFAULT_LORA_WEIGHT,
  editBusy,
  editModels,
  isActiveDownload,
  loraCompatible,
  referenceModel,
  takesReference,
  unsavedEditIds,
  unsavedIds,
  willQueue,
  type EditMode,
  type ImgRef,
  type JobKind,
  type LeaveKind,
  type QueuedJob,
  type TabId,
  type Toast,
} from "./model";
import { importBlob, refFromSession, releaseRefs } from "./images";
import { buildCreateRequest, buildEditRequest, closeRequest, finishRequest, variationRequest } from "./request";
import type { Store } from "./store";
import { canSaveAs, chooseFolder, chooseSavePath, closeWindow, copyText, notifyDone, primeSound, windowInBackground } from "./platform";
import { clearGenerationHandoff } from "../../tabs/create/handoff";

let uidCounter = 0;
const uid = (p: string) => `${p}${Date.now().toString(36)}${(uidCounter++).toString(36)}`;

export type Actions = ReturnType<typeof makeActions>;

export function makeActions(store: Store) {
  const get = store.getState;
  const dispatch = store.dispatch;
  let toastSeq = 0;
  const familyInflight = new Map<string, Promise<FamilyUi>>();

  function toast(text: string, opts: Omit<Toast, "id" | "text"> & { ms?: number } = {}) {
    const id = ++toastSeq;
    const { ms, ...rest } = opts;
    dispatch({ type: "toast", toast: { id, text, ...rest } });
    setTimeout(() => dispatch({ type: "dismissToast", id }), ms ?? 4500);
  }

  // ---------------------------------------------------------------- loading
  async function refreshSettings() {
    dispatch({ type: "setSettings", settings: await api.getSettings() });
  }
  async function refreshModels() {
    const [models, loras] = await Promise.all([api.listModels(), api.listLoras().catch(() => [])]);
    dispatch({ type: "setModels", models });
    dispatch({ type: "setLoras", loras });
  }
  async function refreshStyles() {
    dispatch({ type: "setStyles", styles: await api.listStyles() });
  }
  async function refreshPresets() {
    dispatch({ type: "setPresets", presets: await api.listPresets() });
  }
  async function refreshEngine() {
    dispatch({ type: "setEngine", engine: await api.engineStatus() });
  }
  async function refreshDownloads() {
    dispatch({ type: "setDownloads", list: await api.listDownloads() });
  }

  /** Family UI (cached). */
  function ensureFamilyUi(familyId: string): Promise<FamilyUi> {
    const cached = get().familyUi[familyId];
    if (cached) return Promise.resolve(cached);
    let p = familyInflight.get(familyId);
    if (!p) {
      p = api
        .familyUi(familyId)
        .then((ui) => {
          dispatch({ type: "setFamilyUi", ui });
          return ui;
        })
        .finally(() => familyInflight.delete(familyId));
      familyInflight.set(familyId, p);
    }
    return p;
  }

  // ---------------------------------------------------------------- generation
  function busyError(): CoreError {
    return { code: "invalid", message: "Pinhole is still working on the last image. Wait for it to finish or press Cancel.", details: null };
  }

  const cancelledError = (): CoreError => ({ code: "cancelled", message: "Cancelled.", details: null });

  /** Cancel (or Reset) was pressed during the current job, maybe before the engine had it. */
  let cancelRequested = false;

  /**
   * Mark a job as running for the whole of `work` (only one at a time). Must be called before
   * the caller's first await, so the queue hands over to the next job without a gap.
   */
  async function withJob<T>(kind: JobKind, work: () => Promise<T>, count?: number, imageIds?: string[]): Promise<T> {
    if (get().job) throw busyError();
    cancelRequested = false;
    dispatch({ type: "jobStart", kind, at: Date.now(), count, imageIds });
    let ok = false;
    try {
      const out = await work();
      ok = true;
      return out;
    } catch (e) {
      throw api.asCoreError(e);
    } finally {
      dispatch({ type: "jobEnd" });
      // Finished while the user is elsewhere (and nothing else is waiting): flash the taskbar, chime if asked.
      if (ok && kind !== "describe" && !get().queue.length && windowInBackground()) notifyDone(!!get().settings?.soundOnDone);
      startNextQueued();
    }
  }

  // ---------------------------------------------------------------- queue
  // Generate/Edit/Upscale pressed while a job runs: the job waits here (memory only) and starts when
  // the running one ends. Its promise settles when it has run, so the tab that queued it can
  // show its error. Removed or Reset jobs resolve quietly without running.
  let resetting = false;
  const waiting = new Map<string, { run: () => Promise<void>; resolve: () => void; reject: (e: unknown) => void }>();

  /** Run `run` now if nothing is running, else queue it. `run` must start its job (withJob) before its first await. */
  function enqueue(entry: Omit<QueuedJob, "id">, run: () => Promise<void>): Promise<void> {
    // Pressed while Reset is clearing the session: it belongs to the cleared session.
    if (resetting) return Promise.resolve();
    // The click that started this is the moment a WebView will let the chime's audio start.
    if (get().settings?.soundOnDone) primeSound();
    if (!willQueue(get())) return run();
    const id = uid("q");
    return new Promise<void>((resolve, reject) => {
      waiting.set(id, { run, resolve, reject });
      dispatch({ type: "queueAdd", job: { ...entry, id } });
      toast(get().queue.length === 1 ? "Added to the queue. It starts when the current one finishes." : "Added to the queue.", { ms: 2500 });
    });
  }

  function startNextQueued() {
    if (get().job) return;
    const next = get().queue[0];
    if (!next) return;
    const w = waiting.get(next.id);
    waiting.delete(next.id);
    // Start it before dropping it from the queue: the running job references its images from then on.
    const p = w ? w.run() : Promise.resolve();
    dispatch({ type: "queueRemove", id: next.id });
    if (w) p.then(w.resolve, w.reject);
    else startNextQueued();
  }

  /** Take a waiting job out of the queue (it never runs). */
  function removeQueued(id: string) {
    const w = waiting.get(id);
    waiting.delete(id);
    dispatch({ type: "queueRemove", id });
    w?.resolve();
  }

  function clearQueue() {
    const all = [...waiting.values()];
    waiting.clear();
    for (const q of get().queue) dispatch({ type: "queueRemove", id: q.id });
    for (const w of all) w.resolve();
  }

  /**
   * Blob refs for images a job just made. Images that land after Reset belong to the
   * cleared session: they are released and the job counts as cancelled.
   */
  async function jobRefs(images: ResultImage[], nonce: number): Promise<ImgRef[]> {
    try {
      const refs = await refsFromSession(images);
      if (get().sessionNonce === nonce) return refs;
      releaseRefs(refs, true);
    } catch (e) {
      if (get().sessionNonce === nonce) throw e;
    }
    throw cancelledError();
  }

  /** `nonce`: the session when the job started (steps before this one may have awaited). */
  async function generateNow(req: GenerateRequest, nonce = get().sessionNonce): Promise<{ images: ResultImage[]; refs: ImgRef[] }> {
    if (cancelRequested || get().sessionNonce !== nonce) throw cancelledError();
    const res = await api.generate(req);
    return { images: res.images, refs: await jobRefs(res.images, nonce) };
  }


  /** Blob refs for new session images. If one can't be read, the others are released too and the error is thrown. */
  async function refsFromSession(images: ResultImage[]): Promise<ImgRef[]> {
    const got = await Promise.allSettled(images.map((im) => refFromSession(im.id, im.width, im.height)));
    const failed = got.find((r): r is PromiseRejectedResult => r.status === "rejected");
    if (!failed) return got.map((r) => (r as PromiseFulfilledResult<ImgRef>).value);
    releaseRefs(got.flatMap((r) => (r.status === "fulfilled" ? [r.value] : [])), true);
    for (const im of images) void api.discardImage(im.id).catch(() => undefined);
    throw failed.reason;
  }

  function currentCreateModel(): InstalledModel | null {
    const s = get();
    return (s.models ?? []).find((m) => m.id === s.create.modelId) ?? null;
  }

  /**
   * Generate from the Create tab, with the settings as they are now (queued if a job is
   * running). Resolves when it has run; quietly on cancel.
   */
  async function generateCreate(): Promise<void> {
    const s = get();
    const model = currentCreateModel();
    if (!model) throw { code: "not_found", message: "Pick a model first — or get one of the recommended models.", details: null } as CoreError;
    if (!s.create.prompt.trim()) throw { code: "invalid", message: "Type what you want to see first.", details: null } as CoreError;
    if (s.create.refImageId && !takesReference(model)) {
      const fix = referenceModel(s.models) ? "Switch to a model that can" : "Use it in Edit";
      throw { code: "invalid", message: `${model.friendlyName} can't use a reference picture. ${fix}, or remove the picture.`, details: null } as CoreError;
    }
    const { create, loras, settings } = s;
    const imageIds = create.refImageId ? [create.refImageId] : [];
    await queueBatch(
      create.count,
      queueEntry("create", create.prompt, model, create.count, imageIds),
      async () => {
        dispatch({ type: "pushPrompt", prompt: create.prompt });
        const ui = model.familyId ? await ensureFamilyUi(model.familyId).catch(() => null) : null;
        return buildCreateRequest(create, { ui, loras, model, settings });
      },
    );
  }

  function queueEntry(kind: QueuedJob["kind"], prompt: string, model: InstalledModel | undefined, count: number, imageIds: string[] = []): Omit<QueuedJob, "id"> {
    const text = prompt.trim().replace(/\s+/g, " ");
    const images = kind === "edit" ? "" : ` · ${count} image${count === 1 ? "" : "s"}`;
    return { kind, label: text || (kind === "edit" ? "Edit" : "Picture"), detail: `${model?.friendlyName ?? "Model"}${images}`, imageIds };
  }

  /** A Create batch: `makeRequest` runs once the job has started. */
  async function queueBatch(count: number, entry: Omit<QueuedJob, "id">, makeRequest: () => Promise<GenerateRequest>) {
    try {
      await enqueue(entry, async () => {
        await withJob(
          "create",
          async () => {
            const nonce = get().sessionNonce;
            const req = await makeRequest();
            const { images, refs } = await generateNow(req, nonce);
            // Added while the job still holds its images: the batch keeps its reference picture
            // (for Variations) even if the slot was cleared meanwhile.
            if (images.length) dispatch({ type: "addResults", batch: { id: uid("b"), request: req }, images, refs });
          },
          count,
          entry.imageIds,
        );
        // The model's lastUsed changed; refresh quietly so the picker order stays right.
        void refreshModels().catch(() => undefined);
      });
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code === "cancelled") return;
      throw err;
    }
  }

  function runBatch(req: GenerateRequest) {
    const model = (get().models ?? []).find((m) => m.id === req.modelId);
    const imageIds = [...(req.initImageId ? [req.initImageId] : []), ...(req.refImageIds ?? [])];
    return queueBatch(req.dials.count, queueEntry("create", req.prompt, model, req.dials.count, imageIds), async () => req);
  }

  function batchOf(resultId: string) {
    const s = get();
    const batch = s.batches[s.resultBatch[resultId] ?? ""];
    if (!batch) throw { code: "not_found", message: "The settings for this image are no longer in memory. Generate again from the Create tab.", details: null } as CoreError;
    return batch;
  }

  /** Same prompt and settings, new seeds. */
  async function variations(resultId: string) {
    await runBatch(variationRequest(batchOf(resultId).request));
  }

  /** "Close to this one": How many new pictures that keep this one's layout and change the details. */
  async function closeTo(resultId: string) {
    const batch = batchOf(resultId);
    const result = get().results.find((r) => r.id === resultId);
    if (!result) return;
    const req = closeRequest(batch.request, result);
    await runBatch({ ...req, dials: { ...req.dials, count: get().create.count } });
  }

  /** "Finish at Best quality": this picture again at Best, same seed and settings. */
  async function finishAtBest(resultId: string) {
    const batch = batchOf(resultId);
    const result = get().results.find((r) => r.id === resultId);
    const req = result ? finishRequest(batch.request, result.seed) : null;
    if (req) await runBatch(req);
  }

  /** The queue list's line for an upscale. */
  function upscaleEntry(kind: "upscale" | "editUpscale", imageId: string, factor: 2 | 4, imageIds: string[]): Omit<QueuedJob, "id"> {
    const im = get().images[imageId];
    return { kind, label: `Upscale ${factor}×`, detail: im ? `${im.width * factor}×${im.height * factor}` : "Upscale", imageIds };
  }

  /** Upscale a Create result (queued if a job is running). Resolves when it has run; quietly on cancel. */
  async function upscale(resultId: string, factor: 2 | 4) {
    // Read when pressed: the source (and with it its batch) may be removed while it waits or runs.
    // The queue entry and then the job hold the batch's reference picture, so Variations of the
    // upscale still work.
    const batchId = get().resultBatch[resultId];
    const batch = batchId ? get().batches[batchId] : undefined;
    const imageIds = [resultId, ...(batch?.request.refImageIds ?? [])];
    try {
      await enqueue(upscaleEntry("upscale", resultId, factor, imageIds), () =>
        withJob("upscale", async () => {
          const nonce = get().sessionNonce;
          const im = await api.upscaleImage(resultId, factor);
          // Cancel pressed while the upscaler was still downloading (the engine had no job
          // to stop yet): drop the image instead of adding it.
          if (cancelRequested) {
            void api.discardImage(im.id).catch(() => undefined);
            throw cancelledError();
          }
          const refs = await jobRefs([im], nonce);
          dispatch({ type: "addResults", batch: batch ?? null, images: [im], refs });
        }, 1, imageIds),
      );
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    }
  }

  async function cancel() {
    if (get().job) cancelRequested = true;
    await api.cancelGeneration().catch(() => undefined);
  }

  // ---------------------------------------------------------------- results
  async function save(id: string) {
    const saved = await api.saveImage(id);
    dispatch({ type: "markSaved", entries: [{ id, path: saved.path }] });
    toast(`Saved to ${saved.path}`, { action: { label: "Show folder", run: () => void api.openOutputsFolder() }, ms: 8000 });
    return saved;
  }

  async function saveAs(id: string, seed: number | null) {
    if (!canSaveAs()) return save(id);
    const path = await chooseSavePath(`pinhole_${seed ?? "image"}.png`);
    if (!path) return null;
    const saved = await api.saveImageAs(id, path);
    dispatch({ type: "markSaved", entries: [{ id, path: saved.path }] });
    toast(`Saved to ${saved.path}`);
    return saved;
  }

  /** "Save all": asks for a folder, then saves every unsaved picture (or just `only`) there. False when cancelled; throws if some couldn't be saved. */
  async function saveAll(only?: string[]): Promise<boolean> {
    const ids = only ?? unsavedIds(get());
    if (!ids.length) return true;
    const dir = await chooseFolder("Save all pictures to…");
    if (!dir) return false;
    const batch = await api.saveImagesTo(ids, dir);
    dispatch({ type: "markSaved", entries: batch.saved });
    const n = batch.saved.length;
    if (n) toast(`Saved ${n} ${n === 1 ? "picture" : "pictures"} to ${dir}`, { ms: 8000 });
    if (batch.failed) {
      throw { code: "io", message: `${batch.failed} of ${n + batch.failed} pictures couldn't be saved. Check that the folder can be written to, then try again.`, details: null } as CoreError;
    }
    return true;
  }

  /** The window is closing (or Reset was pressed): true = go ahead; false = unsaved pictures, the question dialog is now showing. */
  function requestLeave(what: "close" | "clear"): boolean {
    if (!unsavedIds(get()).length) return true;
    afterEditReplaced = null;
    dispatch({ type: "askLeave", what });
    return false;
  }

  // What runs when the user goes ahead with replacing Edit's history ("edit" in the dialog).
  let afterEditReplaced: (() => Promise<void> | void) | null = null;

  /**
   * Another image is about to replace Edit's history: true = go ahead. With unsaved edit
   * results, the unsaved-pictures dialog asks first and `then` runs if the user goes ahead.
   */
  function confirmReplaceEdit(then: () => Promise<void> | void): boolean {
    if (!unsavedEditIds(get()).length) return true;
    afterEditReplaced = then;
    dispatch({ type: "askLeave", what: "edit" });
    return false;
  }

  async function finishLeave(what: LeaveKind) {
    dispatch({ type: "askLeave", what: null });
    if (what === "edit") {
      const then = afterEditReplaced;
      afterEditReplaced = null;
      try {
        await then?.();
      } catch (e) {
        toast(api.asCoreError(e).message);
      }
    } else if (what === "close") await closeWindow();
    else await clearSession();
  }

  async function copyImage(id: string) {
    await api.copyImage(id);
    toast("Image copied");
  }

  async function copyTextToClipboard(text: string, what = "Copied") {
    await copyText(text);
    toast(what);
  }

  function setTab(tab: TabId) {
    dispatch({ type: "setTab", tab });
  }

  /** Use an installed style add-on in Create (strength 0.8, adjustable under the prompt). */
  function addLora(loraId: string) {
    const s = get();
    const lora = s.loras.find((l) => l.id === loraId);
    if (!lora) return;
    if (!s.create.loras.some((u) => u.loraId === loraId)) {
      dispatch({ type: "patchCreate", patch: { loras: [...s.create.loras, { loraId, weight: DEFAULT_LORA_WEIGHT }] } });
    }
    setTab("create");
    const model = s.models?.find((m) => m.id === s.create.modelId);
    toast(
      model && !loraCompatible(lora, model.familyId)
        ? `Added “${lora.friendlyName}”, but it's made for ${lora.baseModel ?? "other"} models. Pick one of those to use it.`
        : `Added “${lora.friendlyName}”.`,
    );
  }

  /** Save the user's own trigger words for an add-on. Its chips go back to the default pick. Throws CoreError. */
  async function setLoraTriggerWords(loraId: string, words: string[]) {
    const updated = await api.setLoraTriggerWords(loraId, words);
    const s = get();
    dispatch({ type: "setLoras", loras: s.loras.map((l) => (l.id === loraId ? updated : l)) });
    const reset = (used: LoraUse[]) =>
      used.some((u) => u.loraId === loraId && u.words) ? used.map((u) => (u.loraId === loraId ? { loraId: u.loraId, weight: u.weight } : u)) : null;
    const create = reset(get().create.loras);
    if (create) dispatch({ type: "patchCreate", patch: { loras: create } });
    const edit = reset(get().edit.loras);
    if (edit) dispatch({ type: "patchEdit", patch: { loras: edit } });
  }

  function sendToEdit(id: string, confirmed = false) {
    const ref = get().images[id];
    if (!ref) return;
    if (editBusy(get())) {
      toast("Wait for the edits in progress to finish first.");
      return;
    }
    if (!confirmed && !confirmReplaceEdit(() => sendToEdit(id, true))) return;
    dispatch({ type: "editLoad", ref });
    setTab("edit");
  }

  /** "Use as image 2": opens "Describe a change" with this image as image 2. With no image 1 yet it becomes image 1. */
  function sendToEditSecond(id: string) {
    const ref = get().images[id];
    if (!ref) return;
    if (editBusy(get())) {
      toast("Wait for the edits in progress to finish first.");
      return;
    }
    const edit = get().edit;
    if (edit.chain.length && edit.chain[edit.index]?.imageId === id) {
      toast("That's already image 1. Pick a different image for image 2.");
      return;
    }
    if (!edit.chain.length) {
      dispatch({ type: "editLoad", ref });
      if (edit.secondImageId === id) dispatch({ type: "editSetSecond", ref: null });
      toast("Loaded as image 1. Pick another image to use as image 2.");
    } else {
      dispatch({ type: "editSetSecond", ref });
      dispatch({ type: "patchEdit", patch: { mode: "instruction" } });
    }
    setTab("edit");
  }

  /**
   * "Same character": new pictures of the subject in this image, using what's installed. Create
   * with it as the reference picture when the Create model (or another installed one) can take
   * one, else "Describe a change" in Edit, which offers a one-click edit model when none is
   * installed. The image keeps its id, so its origin follows every result made from it.
   */
  function sameCharacter(id: string, confirmed = false) {
    const s = get();
    const ref = s.images[id];
    if (!ref) return;
    const current = (s.models ?? []).find((m) => m.id === s.create.modelId) ?? null;
    // The picked Create model is used whenever it takes a reference picture, like the reference
    // slot. Only switch to another model that can run now; otherwise Edit, which offers a one-click model.
    const usable = (m: InstalledModel | null) => !!m && takesReference(m) && !m.missingComponents.length && m.fit !== "tooBig";
    const other = referenceModel(s.models);
    const able = takesReference(current) ? current : usable(other) ? other : null;
    if (able) {
      if (able !== current) dispatch({ type: "selectModel", modelId: able.id });
      dispatch({ type: "createSetRef", ref });
      setTab("create");
      toast(
        able === current
          ? "Set as the reference picture. Now describe the new scene, like “the same character on a beach”."
          : `Switched to ${able.friendlyName}, which can use a reference picture. Now describe the new scene, like “the same character on a beach”.`,
        { ms: 7000 },
      );
      return;
    }
    if (editBusy(s)) {
      toast("Wait for the edits in progress to finish first.");
      return;
    }
    if (!confirmed && !confirmReplaceEdit(() => sameCharacter(id, true))) return;
    dispatch({ type: "editLoad", ref });
    dispatch({ type: "patchEdit", patch: { mode: "instruction" } });
    // One picture in, not a two-image combine with a leftover image 2.
    if (s.edit.secondImageId) dispatch({ type: "editSetSecond", ref: null });
    if (s.create.refImageId === id) dispatch({ type: "createSetRef", ref: null });
    setTab("edit");
    toast("Describe the new scene, like “the same character on a beach”.", { ms: 7000 });
  }

  function sendToDescribe(id: string) {
    const ref = get().images[id];
    if (!ref) return;
    dispatch({ type: "describeLoad", ref });
    setTab("describe");
  }

  /** "Use as prompt" (Describe → Create). */
  function useAsPrompt(text: string) {
    dispatch({ type: "patchCreate", patch: { prompt: text } });
    setTab("create");
  }

  function removeResult(id: string) {
    dispatch({ type: "removeResult", id });
  }

  // ---------------------------------------------------------------- import
  /** Reset pressed while an image was being read: it belongs to the cleared session. */
  function importOutlived(ref: ImgRef, nonce: number) {
    if (!resetting && get().sessionNonce === nonce) return false;
    releaseRefs([ref], true);
    return true;
  }

  /**
   * False without loading when the user is first asked about unsaved edits: if they go
   * ahead it loads then and opens the Edit tab.
   */
  async function importToEdit(blob: Blob, confirmed = false): Promise<boolean> {
    if (editBusy(get())) throw busyError();
    if (!confirmed && !confirmReplaceEdit(async () => {
      await importToEdit(blob, true);
      setTab("edit");
    })) return false;
    const nonce = get().sessionNonce;
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    if (importOutlived(ref, nonce)) return true;
    // An edit started while the image was being read: keep its history.
    if (editBusy(get())) {
      releaseRefs([ref], true);
      throw busyError();
    }
    dispatch({ type: "editLoad", ref });
    return true;
  }

  /** Create's optional reference picture. */
  async function importCreateReference(blob: Blob) {
    const nonce = get().sessionNonce;
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    if (importOutlived(ref, nonce)) return;
    dispatch({ type: "createSetRef", ref });
  }
  /** The optional second image for "Describe a change". */
  async function importSecondToEdit(blob: Blob) {
    if (editBusy(get())) throw busyError();
    const nonce = get().sessionNonce;
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    if (importOutlived(ref, nonce)) return;
    if (editBusy(get())) {
      releaseRefs([ref], true);
      throw busyError();
    }
    dispatch({ type: "editSetSecond", ref });
  }



  async function importToDescribe(blob: Blob) {
    const nonce = get().sessionNonce;
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    if (importOutlived(ref, nonce)) return;
    dispatch({ type: "describeLoad", ref });
  }

  // ---------------------------------------------------------------- edit
  /**
   * Edit a history step with the settings as they are now (queued if a job is running).
   * `from`: the step to edit (default: the shown one); the result replaces every later step.
   * `newSeed`: ignore a fixed Seed ("Try again").
   */
  async function runEdit(opts: { mode: EditMode; model: InstalledModel; mask: Blob | null; size: [number, number]; from?: number; newSeed?: boolean }) {
    const s = get();
    const shown = s.edit.index;
    const at = opts.from ?? shown;
    const node = s.edit.chain[at];
    const source = node ? s.images[node.imageId] : undefined;
    if (!node || !source) throw { code: "invalid", message: "Add an image to edit first.", details: null } as CoreError;
    const text =
      opts.mode === "instruction" ? s.edit.instruction : opts.mode === "fix" ? s.edit.fixPrompt : opts.mode === "extend" ? s.edit.extendPrompt : s.edit.restylePrompt;
    if (opts.mode !== "fix" && opts.mode !== "extend" && !text.trim() && !s.edit.styleId) {
      throw {
        code: "invalid",
        message: opts.mode === "instruction" ? "Say what should change first." : "Describe how it should look (or pick a style) first.",
        details: null,
      } as CoreError;
    }
    const { edit, loras, settings } = s;
    const second = opts.mode === "instruction" && edit.secondImageId ? [edit.secondImageId] : [];
    let maskId: string | null = null;
    try {
      await enqueue(queueEntry("edit", text, opts.model, 1, [source.id, ...second]), () =>
        // The job (and with it the history lock) starts before the first await.
        withJob(
          "edit",
          async () => {
            const nonce = get().sessionNonce;
            const ui = opts.model.familyId ? await ensureFamilyUi(opts.model.familyId).catch(() => null) : null;
            if (opts.mask) maskId = (await api.importImage(new Uint8Array(await opts.mask.arrayBuffer()))).id;
            const req = buildEditRequest(opts.newSeed ? { ...edit, seed: null } : edit, {
              mode: opts.mode,
              source,
              model: opts.model,
              ui,
              maskImageId: maskId,
              size: opts.size,
              loras,
              autoAdd: settings?.addTriggerWords ?? true,
            });
            // Checked here, with the model's own shape sizes.
            if (opts.mode === "extend" && !req.extend) {
              throw { code: "invalid", message: "The picture is already this shape. Pick another shape to extend it.", details: null } as CoreError;
            }
            const { images, refs } = await generateNow(req, nonce);
            // Added after the step it was made from, replacing later steps. A queued edit of an
            // earlier step goes at the end instead, keeping the edits made since; if that step is
            // gone, the result is dropped.
            const now = get().edit;
            const pos = now.chain.findIndex((n) => n.imageId === node.imageId);
            if (refs[0] && pos >= 0) {
              const secondImageId = second[0];
              if (opts.from != null || pos === now.index) dispatch({ type: "editPush", ref: refs[0], meta: images[0] ?? null, after: pos, secondImageId });
              else dispatch({ type: "editAppend", ref: refs[0], meta: images[0] ?? null, secondImageId });
              releaseRefs(refs.slice(1), true);
            } else {
              releaseRefs(refs, true);
              if (refs[0]) toast("The edit finished after the image changed, so it wasn't added.");
            }
          },
          1,
          [source.id, ...second],
        ).then(() => undefined),
      );
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    } finally {
      if (maskId) void api.discardImage(maskId).catch(() => undefined);
    }
  }

  /**
   * Upscale the shown edit step (queued if a job is running); the result becomes the next step.
   * Like a queued edit, one that waited while the history moved on is added at the end, and
   * dropped if its step is gone. Resolves quietly on cancel.
   */
  async function upscaleEdit(factor: 2 | 4) {
    const s = get();
    const node = s.edit.chain[s.edit.index];
    if (!node) throw { code: "invalid", message: "Add an image to edit first.", details: null } as CoreError;
    try {
      await enqueue(upscaleEntry("editUpscale", node.imageId, factor, [node.imageId]), () =>
        // The job (and with it the history lock) starts before the first await.
        withJob("editUpscale", async () => {
          const nonce = get().sessionNonce;
          const im = await api.upscaleImage(node.imageId, factor);
          if (cancelRequested) {
            void api.discardImage(im.id).catch(() => undefined);
            throw cancelledError();
          }
          const refs = await jobRefs([im], nonce);
          const now = get().edit;
          const pos = now.chain.findIndex((n) => n.imageId === node.imageId);
          if (pos >= 0 && pos === now.index) dispatch({ type: "editPush", ref: refs[0], meta: im, after: pos });
          else if (pos >= 0) dispatch({ type: "editAppend", ref: refs[0], meta: im });
          else {
            releaseRefs(refs, true);
            toast("The upscale finished after the image changed, so it wasn't added.");
          }
        }, 1, [node.imageId]),
      );
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    }
  }

  /** Best installed edit model; with `twoImages`, only ones that combine two images. */
  function autoEditModel(twoImages = false): InstalledModel | null {
    const list = editModels(get().models, twoImages).filter((m) => !m.missingComponents.length);
    const all = list.length ? list : editModels(get().models, twoImages);
    // Qwen-Image 2.1 (the recommended edit model, it also creates) first, then the dedicated
    // edit models, then other generators that can edit (FLUX.2).
    const rank = (m: InstalledModel) =>
      m.familyId === "qwen_image_21" ? 0 : m.familyId === "qwen_image_edit_2511" ? 1 : m.familyId === "flux1_kontext" ? 2 : m.isEditModel ? 3 : 4;
    const fitRank = (m: InstalledModel) => (m.fit === "fits" ? 0 : m.fit === "tight" ? 1 : m.fit === "tooBig" ? 3 : 2);
    // One image: the models ranked above generators first unless they're too big.
    // Two images need more memory, so a model that fits wins (FLUX.2 klein over a tight Qwen Edit).
    const tier = (m: InstalledModel) => (twoImages ? fitRank(m) : rank(m) < 4 && m.fit !== "tooBig" ? 0 : 1);
    return [...all].sort((a, b) => tier(a) - tier(b) || fitRank(a) - fitRank(b) || rank(a) - rank(b))[0] ?? null;
  }

  // ---------------------------------------------------------------- session
  /** The Reset button: asks first when there are unsaved pictures. */
  async function clearSessionChecked() {
    if (requestLeave("clear")) await clearSession();
  }

  async function clearSession() {
    resetting = true;
    afterEditReplaced = null;
    try {
      clearQueue();
      if (get().job) await cancel();
      await api.clearSession().catch(() => undefined);
      clearGenerationHandoff();
      dispatch({ type: "clearSession" });
    } finally {
      resetting = false;
    }
    toast("Reset: prompt fields and unsaved images were cleared.");
  }

  function onEngine(engine: EngineStatus) {
    dispatch({ type: "setEngine", engine });
  }

  function activeDownloads() {
    return get().downloads.filter(isActiveDownload);
  }

  function clearFinishedDownloads() {
    dispatch({ type: "clearFinishedDownloads" });
  }

  return {
    toast,
    refreshSettings,
    refreshModels,
    refreshStyles,
    refreshPresets,
    refreshEngine,
    refreshDownloads,
    ensureFamilyUi,
    generateCreate,
    runBatch,
    removeQueued,
    variations,
    closeTo,
    finishAtBest,
    upscale,
    cancel,
    save,
    saveAs,
    saveAll,
    requestLeave,
    finishLeave,
    copyImage,
    copyTextToClipboard,
    setTab,
    addLora,
    setLoraTriggerWords,
    sendToEdit,
    sendToEditSecond,
    sameCharacter,
    sendToDescribe,
    useAsPrompt,
    removeResult,
    importToEdit,
    importCreateReference,
    importSecondToEdit,
    importToDescribe,
    runEdit,
    upscaleEdit,
    autoEditModel,
    clearSession,
    clearSessionChecked,
    onEngine,
    activeDownloads,
    clearFinishedDownloads,
    currentCreateModel,
  };
}
