// Async app actions (IPC + state updates). Components call these; errors are
// thrown as CoreError for the caller to show with <ErrorNotice>.
//
// PRIVACY: prompt text passes through here into IPC only. Never log it.

import * as api from "../api";
import type { CoreError, EngineStatus, FamilyUi, GenerateRequest, InstalledModel, ResultImage } from "../types";
import { DEFAULT_LORA_WEIGHT, editModels, isActiveDownload, loraCompatible, type EditMode, type ImgRef, type JobKind, type TabId, type Toast } from "./model";
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

  /** Mark a job as running for the whole of `work` (only one at a time). */
  async function withJob<T>(kind: JobKind, work: () => Promise<T>, count?: number): Promise<T> {
    if (get().job) throw busyError();
    cancelRequested = false;
    dispatch({ type: "jobStart", kind, at: Date.now(), count });
    try {
      return await work();
    } catch (e) {
      throw api.asCoreError(e);
    } finally {
      dispatch({ type: "jobEnd" });
    }
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

  const runJob = (kind: JobKind, req: GenerateRequest) => withJob(kind, () => generateNow(req), req.dials.count);

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

  /** Generate from the Create tab. Resolves quietly on cancel. */
  async function generateCreate(): Promise<void> {
    const s = get();
    const model = currentCreateModel();
    if (!model) throw { code: "not_found", message: "Pick a model first — or get one of the recommended models.", details: null } as CoreError;
    if (!s.create.prompt.trim()) throw { code: "invalid", message: "Type what you want to see first.", details: null } as CoreError;
    const ui = model.familyId ? await ensureFamilyUi(model.familyId).catch(() => null) : null;
    const req = buildCreateRequest(s.create, { ui, loras: s.loras, model, settings: s.settings });
    await runBatch(req);
  }

  async function runBatch(req: GenerateRequest) {
    try {
      const { images, refs } = await runJob("create", req);
      if (images.length) dispatch({ type: "addResults", batch: { id: uid("b"), request: req }, images, refs });
      // The model's lastUsed changed; refresh quietly so the picker order stays right.
      void refreshModels().catch(() => undefined);
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code === "cancelled") return;
      throw err;
    }
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
    const used = get().create.loras;
    if (used.some((u) => u.loraId === loraId && u.words)) {
      dispatch({ type: "patchCreate", patch: { loras: used.map((u) => (u.loraId === loraId ? { loraId: u.loraId, weight: u.weight } : u)) } });
    }
  }

  function sendToEdit(id: string) {
    const ref = get().images[id];
    if (!ref) return;
    if (get().job?.kind === "edit") {
      toast("Wait for the current edit to finish first.");
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
    if (get().job?.kind === "edit") throw busyError();
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    // An edit started while the image was being read: keep its history.
    if (get().job?.kind === "edit") {
      releaseRefs([ref], true);
      throw busyError();
    }
    dispatch({ type: "editLoad", ref });
  }

  async function importToDescribe(blob: Blob) {
    const ref = await importBlob(blob).catch((e) => {
      throw api.asCoreError(e);
    });
    dispatch({ type: "describeLoad", ref });
  }

  // ---------------------------------------------------------------- edit
  async function runEdit(opts: { mode: EditMode; model: InstalledModel; mask: Blob | null; size: [number, number] }) {
    const s = get();
    const node = s.edit.chain[s.edit.index];
    const source = node ? s.images[node.imageId] : undefined;
    if (!source) throw { code: "invalid", message: "Add an image to edit first.", details: null } as CoreError;
    const text = opts.mode === "instruction" ? s.edit.instruction : s.edit.restylePrompt;
    if (!text.trim() && !s.edit.styleId) {
      throw {
        code: "invalid",
        message: opts.mode === "instruction" ? "Say what should change first." : "Describe how it should look (or pick a style) first.",
        details: null,
      } as CoreError;
    }
    let maskId: string | null = null;
    try {
      // The job (and with it the history lock) starts before the first await.
      await withJob("edit", async () => {
        const nonce = get().sessionNonce;
        const ui = opts.model.familyId ? await ensureFamilyUi(opts.model.familyId).catch(() => null) : null;
        if (opts.mask) maskId = (await api.importImage(new Uint8Array(await opts.mask.arrayBuffer()))).id;
        const req = buildEditRequest(get().edit, { mode: opts.mode, source, model: opts.model, ui, maskImageId: maskId, size: opts.size });
        const { images, refs } = await generateNow(req, nonce);
        // The history is locked while an edit runs; if the image on screen changed anyway,
        // don't attach the result to another image's history.
        const now = get().edit;
        if (refs[0] && now.chain[now.index]?.imageId === node.imageId) {
          dispatch({ type: "editPush", ref: refs[0], meta: images[0] ?? null });
          releaseRefs(refs.slice(1), true);
        } else {
          releaseRefs(refs, true);
          if (refs[0]) toast("The edit finished after the image changed, so it wasn't added.");
        }
      });
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    } finally {
      if (maskId) void api.discardImage(maskId).catch(() => undefined);
    }
  }

  /** The edit model Pinhole picks automatically (the registry lists edit families best-first). */
  function autoEditModel(): InstalledModel | null {
    const list = editModels(get().models).filter((m) => !m.missingComponents.length);
    const all = list.length ? list : editModels(get().models);
    const rank = (m: InstalledModel) => (m.familyId === "qwen_image_edit_2511" ? 0 : m.familyId === "flux1_kontext" ? 1 : 2);
    const fitRank = (m: InstalledModel) => (m.fit === "fits" ? 0 : m.fit === "tight" ? 1 : m.fit === "tooBig" ? 3 : 2);
    // Dedicated edit models first unless they're too big; then generators that can edit (FLUX.2).
    const tier = (m: InstalledModel) => (m.isEditModel && m.fit !== "tooBig" ? 0 : 1);
    return [...all].sort((a, b) => tier(a) - tier(b) || fitRank(a) - fitRank(b) || rank(a) - rank(b))[0] ?? null;
  }

  // ---------------------------------------------------------------- session
  async function clearSession() {
    if (get().job) await cancel();
    await api.clearSession().catch(() => undefined);
    clearGenerationHandoff();
    dispatch({ type: "clearSession" });
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
    importToDescribe,
    runEdit,
    autoEditModel,
    clearSession,
    onEngine,
    activeDownloads,
    clearFinishedDownloads,
    currentCreateModel,
  };
}
