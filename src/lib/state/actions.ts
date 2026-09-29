// Async app actions (IPC + state updates). Components call these; errors are
// thrown as CoreError for the caller to show with <ErrorNotice>.
//
// PRIVACY: prompt text passes through here into IPC only. Never log it.

import * as api from "../api";
import type { CoreError, EngineStatus, FamilyUi, GenerateRequest, InstalledModel, ResultImage } from "../types";
import { editModels, isActiveDownload, type EditMode, type ImgRef, type JobKind, type TabId, type Toast } from "./model";
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

  async function runJob(kind: JobKind, req: GenerateRequest): Promise<{ images: ResultImage[]; refs: ImgRef[] }> {
    if (get().job) throw busyError();
    dispatch({ type: "jobStart", kind, at: Date.now() });
    const nonce = get().sessionNonce;
    try {
      const res = await api.generate(req);
      const refs = await refsFromSession(res.images);
      // Reset while the job was finishing: the images belong to the cleared session.
      if (get().sessionNonce !== nonce) {
        releaseRefs(refs, true);
        throw { code: "cancelled", message: "Cancelled.", details: null } as CoreError;
      }
      return { images: res.images, refs };
    } catch (e) {
      throw api.asCoreError(e);
    } finally {
      dispatch({ type: "jobEnd" });
    }
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
    if (get().job) throw busyError();
    dispatch({ type: "jobStart", kind: "upscale", at: Date.now() });
    const nonce = get().sessionNonce;
    try {
      const im = await api.upscaleImage(resultId, factor);
      const ref = await refFromSession(im.id, im.width, im.height);
      if (get().sessionNonce !== nonce) {
        releaseRefs([ref], true);
        return;
      }
      const batchId = get().resultBatch[resultId];
      const batch = batchId ? get().batches[batchId] : undefined;
      dispatch({ type: "addResults", batch: batch ?? null, images: [im], refs: [ref] });
    } catch (e) {
      const err = api.asCoreError(e);
      if (err.code !== "cancelled") throw err;
    } finally {
      dispatch({ type: "jobEnd" });
    }
  }

  async function cancel() {
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
    const ui = opts.model.familyId ? await ensureFamilyUi(opts.model.familyId).catch(() => null) : null;
    let maskId: string | null = null;
    try {
      if (opts.mask) maskId = (await api.importImage(new Uint8Array(await opts.mask.arrayBuffer()))).id;
      const req = buildEditRequest(get().edit, { mode: opts.mode, source, model: opts.model, ui, maskImageId: maskId, size: opts.size });
      const { images, refs } = await runJob("edit", req);
      // The history is locked while an edit runs; if the image on screen changed anyway
      // (e.g. a new image was loaded from another tab), don't attach the result to it.
      const now = get().edit;
      if (refs[0] && now.chain[now.index]?.imageId === node.imageId) {
        dispatch({ type: "editPush", ref: refs[0], meta: images[0] ?? null });
        releaseRefs(refs.slice(1), true);
      } else {
        releaseRefs(refs, true);
      }
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
    return [...all].sort((a, b) => fitRank(a) - fitRank(b) || rank(a) - rank(b))[0] ?? null;
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
