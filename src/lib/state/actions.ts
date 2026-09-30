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
  takesReference,
  willQueue,
  type EditMode,
  type ImgRef,
  type JobKind,
  type QueuedJob,
  type TabId,
  type Toast,
} from "./model";
import { importBlob, refFromSession, releaseRefs } from "./images";
import { buildCreateRequest, buildEditRequest, variationRequest } from "./request";
import type { Store } from "./store";
import { canSaveAs, chooseSavePath, copyText } from "./platform";
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
    try {
      return await work();
    } catch (e) {
      throw api.asCoreError(e);
    } finally {
      dispatch({ type: "jobEnd" });
      startNextQueued();
    }
  }

  // ---------------------------------------------------------------- queue
  // Generate/Edit pressed while a job runs: the job waits here (memory only) and starts when
  // the running one ends. Its promise settles when it has run, so the tab that queued it can
  // show its error. Removed or Reset jobs resolve quietly without running.
  let resetting = false;
  const waiting = new Map<string, { run: () => Promise<void>; resolve: () => void; reject: (e: unknown) => void }>();

  /** Run `run` now if nothing is running, else queue it. `run` must start its job (withJob) before its first await. */
  function enqueue(entry: Omit<QueuedJob, "id">, run: () => Promise<void>): Promise<void> {
    // Pressed while Reset is clearing the session: it belongs to the cleared session.
    if (resetting) return Promise.resolve();
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
      throw { code: "invalid", message: `${model.friendlyName} can't use a reference picture. Switch to a model that can, or remove the picture.`, details: null } as CoreError;
    }
    const { create, loras, settings } = s;
    const imageIds = create.refImageId ? [create.refImageId] : [];
    await queueBatch(
      create.count,
      queueEntry("create", create.prompt, model, create.count, imageIds),
      async () => {
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
        const { req, images, refs } = await withJob(
          "create",
          async () => {
            const nonce = get().sessionNonce;
            const req = await makeRequest();
            return { req, ...(await generateNow(req, nonce)) };
          },
          count,
          entry.imageIds,
        );
        if (images.length) dispatch({ type: "addResults", batch: { id: uid("b"), request: req }, images, refs });
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
    return queueBatch(req.dials.count, queueEntry("create", req.prompt, model, req.dials.count, req.refImageIds ?? []), async () => req);
  }

  /** Same prompt and settings, new seeds. */
  async function variations(resultId: string) {
    const s = get();
    const batch = s.batches[s.resultBatch[resultId] ?? ""];
    if (!batch) throw { code: "not_found", message: "The settings for this image are no longer in memory. Generate again from the Create tab.", details: null } as CoreError;
    await runBatch(variationRequest(batch.request));
  }

  async function upscale(resultId: string, factor: 2 | 4) {
    try {
      await withJob("upscale", async () => {
        const nonce = get().sessionNonce;
        const im = await api.upscaleImage(resultId, factor);
        // Cancel pressed while the upscaler was still downloading (the engine had no job
        // to stop yet): drop the image instead of adding it.
        if (cancelRequested) {
          void api.discardImage(im.id).catch(() => undefined);
          throw cancelledError();
        }
        const refs = await jobRefs([im], nonce);
        const batchId = get().resultBatch[resultId];
        const batch = batchId ? get().batches[batchId] : undefined;
        dispatch({ type: "addResults", batch: batch ?? null, images: [im], refs });
      }, 1);
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
    toast(`Saved to ${saved.path}`, { action: { label: "Show folder", run: () => void api.openOutputsFolder() }, ms: 8000 });
    return saved;
  }

  async function saveAs(id: string, seed: number | null) {
    if (!canSaveAs()) return save(id);
    const path = await chooseSavePath(`pinhole_${seed ?? "image"}.png`);
    if (!path) return null;
    const saved = await api.saveImageAs(id, path);
    toast(`Saved to ${saved.path}`);
    return saved;
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

  function sendToEdit(id: string) {
    const ref = get().images[id];
    if (!ref) return;
    if (editBusy(get())) {
      toast("Wait for the edits in progress to finish first.");
      return;
    }
    dispatch({ type: "editLoad", ref });
    setTab("edit");
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
  async function importToEdit(blob: Blob) {
    if (editBusy(get())) throw busyError();
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    // An edit started while the image was being read: keep its history.
    if (editBusy(get())) {
      releaseRefs([ref], true);
      throw busyError();
    }
    dispatch({ type: "editLoad", ref });
  }

  /** Create's optional reference picture. */
  async function importCreateReference(blob: Blob) {
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    dispatch({ type: "createSetRef", ref });
  }

  /** The optional second image for "Describe a change". */
  async function importSecondToEdit(blob: Blob) {
    if (editBusy(get())) throw busyError();
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    if (editBusy(get())) {
      releaseRefs([ref], true);
      throw busyError();
    }
    dispatch({ type: "editSetSecond", ref });
  }

  async function importToDescribe(blob: Blob) {
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
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
    const text = opts.mode === "instruction" ? s.edit.instruction : opts.mode === "fix" ? s.edit.fixPrompt : s.edit.restylePrompt;
    if (opts.mode === "fix" && !opts.mask) {
      throw { code: "invalid", message: "Paint over the spot to fix first.", details: null } as CoreError;
    }
    if (opts.mode !== "fix" && !text.trim() && !s.edit.styleId) {
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
            const { images, refs } = await generateNow(req, nonce);
            // Added after the step it was made from, replacing later steps. A queued edit of an
            // earlier step goes at the end instead, keeping the edits made since; if that step is
            // gone, the result is dropped.
            const now = get().edit;
            const pos = now.chain.findIndex((n) => n.imageId === node.imageId);
            if (refs[0] && pos >= 0) {
              if (opts.from != null || pos === now.index) dispatch({ type: "editPush", ref: refs[0], meta: images[0] ?? null, after: pos });
              else dispatch({ type: "editAppend", ref: refs[0], meta: images[0] ?? null });
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

  /** Upscale the shown edit step; the result becomes the next step. Resolves quietly on cancel. */
  async function upscaleEdit(factor: 2 | 4) {
    const s = get();
    const shown = s.edit.index;
    const node = s.edit.chain[shown];
    if (!node) throw { code: "invalid", message: "Add an image to edit first.", details: null } as CoreError;
    try {
      await withJob("editUpscale", async () => {
        const nonce = get().sessionNonce;
        const im = await api.upscaleImage(node.imageId, factor);
        if (cancelRequested) {
          void api.discardImage(im.id).catch(() => undefined);
          throw cancelledError();
        }
        const refs = await jobRefs([im], nonce);
        const now = get().edit;
        if (now.index === shown && now.chain[shown]?.imageId === node.imageId) {
          dispatch({ type: "editPush", ref: refs[0], meta: im, after: shown });
        } else {
          releaseRefs(refs, true);
          toast("The upscale finished after the image changed, so it wasn't added.");
        }
      }, 1);
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    }
  }

  /** The edit model Pinhole picks automatically (the registry lists edit families best-first). */
  /** Best installed edit model; with `twoImages`, only ones that combine two images. */
  function autoEditModel(twoImages = false): InstalledModel | null {
    const list = editModels(get().models, twoImages).filter((m) => !m.missingComponents.length);
    const all = list.length ? list : editModels(get().models, twoImages);
    const rank = (m: InstalledModel) => (m.familyId === "qwen_image_edit_2511" ? 0 : m.familyId === "flux1_kontext" ? 1 : 2);
    const fitRank = (m: InstalledModel) => (m.fit === "fits" ? 0 : m.fit === "tight" ? 1 : m.fit === "tooBig" ? 3 : 2);
    // One image: dedicated edit models first unless they're too big; then generators that can edit (FLUX.2).
    // Two images need more memory, so a model that fits wins (FLUX.2 klein over a tight Qwen Edit).
    const tier = (m: InstalledModel) => (twoImages ? fitRank(m) : m.isEditModel && m.fit !== "tooBig" ? 0 : 1);
    return [...all].sort((a, b) => tier(a) - tier(b) || fitRank(a) - fitRank(b) || Number(b.isEditModel) - Number(a.isEditModel) || rank(a) - rank(b))[0] ?? null;
  }

  // ---------------------------------------------------------------- session
  async function clearSession() {
    resetting = true;
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
    upscale,
    cancel,
    save,
    saveAs,
    copyImage,
    copyTextToClipboard,
    setTab,
    addLora,
    setLoraTriggerWords,
    sendToEdit,
    sendToDescribe,
    useAsPrompt,
    removeResult,
    importToEdit,
    importSecondToEdit,
    importCreateReference,
    importToDescribe,
    runEdit,
    upscaleEdit,
    autoEditModel,
    clearSession,
    onEngine,
    activeDownloads,
    clearFinishedDownloads,
    currentCreateModel,
  };
}
