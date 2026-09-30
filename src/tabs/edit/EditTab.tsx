// Edit (img2img + instruction editing), SPEC §5.2.
// The image lives in the Rust session (RAM); the edit chain is an in-memory undo stack.
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import {
  ArrowRight,
  Brush,
  ChevronDown,
  Columns2,
  Copy,
  Eraser,
  ImagePlus,
  ListPlus,
  Maximize2,
  Redo2,
  RefreshCw,
  ScanText,
  SlidersHorizontal,
  Trash,
  Trash2,
  Undo2,
  WandSparkles,
} from "lucide-react";
import {
  DropTarget,
  DropZone,
  useFilePicker,
  useImagePaste,
} from "../../components/ImageDrop";
import { CheckReadings } from "../../components/CheckReadings";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { SaveButton, UpscaleMenu } from "../../components/ImageActions";
import { LiveJobProgress } from "../../components/JobProgress";
import { QueueButton } from "../../components/QueueButton";
import { ModelPicker } from "../../components/ModelPicker";
import { StylePicker } from "../../components/StylePicker";
import { ImageViewer } from "../../components/ImageViewer";
import {
  AutoTextarea,
  Button,
  IconButton,
  Kbd,
  Segmented,
  Slider,
  Spinner,
  Toggle,
  cx,
  focusRing,
  inputClass,
} from "../../components/ui";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { useHardware } from "../models/lib/hooks";
import { isCpuOnly } from "../models/lib/words";
import * as api from "../../lib/api";
import { defaultStayClosePosition, sizeMultiple } from "../../lib/paste/map";
import type { CoreError, ExtendCanvas, Quality } from "../../lib/types";
import { useShortcuts } from "../../lib/shortcuts";
import { useActions, usePrimaryAction } from "../../lib/state/AppProvider";
import { useFamilyUi, useModel } from "../../lib/state/hooks";
import {
  createModels,
  editBusy,
  editModels,
  isEditJob,
  willQueue,
  type ChangeAmount,
  type EditMode,
} from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import {
  buildEditRequest,
  editOutputSize,
  extendCanvas,
  settingsSummary,
  type EditSizeChoice,
} from "../../lib/state/request";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import { AddonChips } from "../create/AddonChips";
import { LoraSection, PromptPreview } from "../create/FineTune";
import { CompareView } from "./CompareView";
import { ExtendControls } from "./ExtendControls";
import { MaskCanvas, type MaskHandle } from "./MaskCanvas";
import { useFitBox } from "./useFitBox";

type SizeChoice = EditSizeChoice;

// Recommended edit models that can take a second image (registry `multi_ref`; FLUX.2 has no one-click download yet).
const TWO_IMAGE_PICKS = ["qwen_image_edit_2511"];

const EDIT_JOBS = ["edit", "editUpscale"] as const;

export function EditTab() {
  const tab = useAppState((s) => s.tab);
  const e = useAppState((s) => s.edit);
  const images = useAppState((s) => s.images);
  const models = useAppState((s) => s.models);
  const createModelId = useAppState((s) => s.create.modelId);
  // Only the kind: the progress card subscribes to the job itself (LiveJobProgress).
  const jobKind = useAppState((s) => s.job?.kind ?? null);
  const job = !!jobKind;
  // An edit running or waiting: the history stays put until they are done.
  const locked = useAppState(editBusy);
  const queues = useAppState(willQueue);
  const dispatch = useDispatch();
  const store = useStore();
  const actions = useActions();

  const [error, setError] = useState<CoreError | null>(null);
  const [importing, setImporting] = useState(false);
  const [maskToggle, setMaskOn] = useState(false);
  const [brush, setBrush] = useState(40);
  const [erase, setErase] = useState(false);
  const [painted, setPainted] = useState(false);
  const [compare, setCompare] = useState(false);
  const [compareWith, setCompareWith] = useState<"previous" | "original">(
    "previous",
  );
  const [size, setSize] = useState<SizeChoice>("normal");
  const [moreOpen, setMoreOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const mask = useRef<MaskHandle>(null);
  // The mask each edit was made with (by result image), so "Try again" repaints the same area.
  const masks = useRef(new Map<string, Blob | null>());
  const hw = useHardware();
  const noGpu = !!hw?.detected && isCpuOnly(hw);

  const creates = useMemo(() => createModels(models), [models]);
  // `models` is the real dependency: autoEditModel() reads the current list from the store.
  const autoEditOne = useMemo(
    () => (models ? actions.autoEditModel() : null),
    [actions, models],
  );
  const mode: EditMode = e.mode ?? (autoEditOne ? "instruction" : "restyle");
  // Fix details always uses the brush; Extend never does.
  const fixing = mode === "fix";
  const extending = mode === "extend";
  const maskOn = fixing || (maskToggle && !extending);
  // "Add another image": only models that combine two images (Qwen Image Edit, FLUX.2).
  const second = e.secondImageId ? images[e.secondImageId] : undefined;
  const twoImages = mode === "instruction" && !!second;
  const edits = useMemo(
    () => editModels(models, twoImages),
    [models, twoImages],
  );
  const autoEdit = useMemo(
    () =>
      twoImages ? (models ? actions.autoEditModel(true) : null) : autoEditOne,
    [actions, models, twoImages, autoEditOne],
  );
  const editModelId =
    (e.editModelId && edits.some((m) => m.id === e.editModelId)
      ? e.editModelId
      : null) ??
    autoEdit?.id ??
    null;
  const editFit = edits.find((m) => m.id === editModelId)?.fit ?? null;
  const restyleModelId =
    e.restyleModelId ?? createModelId ?? creates[0]?.id ?? null;
  const model = useModel(mode === "instruction" ? editModelId : restyleModelId);
  const ui = useFamilyUi(model?.familyId);

  const node = e.chain[e.index] ?? null;
  const current = node ? images[node.imageId] : undefined;
  const prevNode =
    compareWith === "original" ? e.chain[0] : e.chain[e.index - 1];
  const before = e.index > 0 && prevNode ? images[prevNode.imageId] : undefined;
  const outSize = current
    ? editOutputSize(
        current.width,
        current.height,
        size,
        sizeMultiple(model?.familyId),
      )
    : null;
  const myJob = isEditJob(jobKind);
  const canvas =
    extending && current
      ? extendCanvas(
          current.width,
          current.height,
          e.extendTo,
          e.extendSide,
          ui,
        )
      : null;

  const load = async (f: File) => {
    // Loading another image mid-edit would attach the result to the wrong history.
    if (editBusy(store.getState())) return;
    setError(null);
    setImporting(true);
    try {
      await actions.importToEdit(f);
      setCompare(false);
    } catch (err) {
      setError(api.asCoreError(err));
    } finally {
      setImporting(false);
    }
  };
  useImagePaste(tab === "edit", (f) => void load(f));
  const picker = useFilePicker((f) => void load(f));

  const loadSecond = async (f: File) => {
    if (editBusy(store.getState())) return;
    setError(null);
    setImporting(true); // Apply waits for image 2
    try {
      await actions.importSecondToEdit(f);
    } catch (err) {
      setError(api.asCoreError(err));
    } finally {
      setImporting(false);
    }
  };
  const secondPicker = useFilePicker((f) => void loadSecond(f));

  // A new current image means a new mask.
  useEffect(() => {
    mask.current?.clear();
  }, [node?.imageId]);
  // Another image (or Reset) starts a new history.
  const originalId = e.chain[0]?.imageId;
  useEffect(() => {
    masks.current.clear();
  }, [originalId]);

  // Set before the first await (the mask export), so a second click or Ctrl+Enter
  // during the export doesn't get as far as the job and report "still working".
  const running = useRef(false);
  // What "Retry" on an error repeats (the last edit, Try again, upscale or save).
  // Kept as data, not a closure, so Retry runs with the current screen's state.
  const retry = useRef<
    | { kind: "edit"; again: boolean }
    | { kind: "upscale"; factor: 2 | 4 }
    | { kind: "action"; f: () => Promise<unknown> }
  >({ kind: "edit", again: false });
  // `again`: redo the shown edit from the step before it, with a new seed ("Try again").
  const run = async (again = false) => {
    if (running.current || importing || !current || !model) return;
    const from = again ? e.index - 1 : e.index;
    const source = e.chain[from] ? images[e.chain[from].imageId] : undefined;
    if (!source || (again && !canTryAgain)) return;
    running.current = true;
    retry.current = { kind: "edit", again };
    setError(null);
    try {
      const m = again
        ? (masks.current.get(current.id) ?? null)
        : maskOn && painted && !twoImages
          ? ((await mask.current?.exportPng()) ?? null)
          : null;
      if (fixing && !m) {
        setError({
          code: "invalid",
          message: "Paint over the spot to fix first.",
          details: null,
        });
        return;
      }
      const outFrom = editOutputSize(
        source.width,
        source.height,
        size,
        sizeMultiple(model.familyId),
      );
      // Queued or started by now: the next press may queue another edit.
      const done = actions.runEdit({
        mode,
        model,
        mask: m,
        size: outFrom,
        from,
        newSeed: again,
      });
      running.current = false;
      await done;
      const now = store.getState().edit;
      const made = now.chain[now.index];
      if (now.index === from + 1 && made && made.imageId !== current.id) {
        masks.current.set(made.imageId, m);
        setCompare(true);
        setCompareWith("previous");
      }
    } catch (err) {
      setError(api.asCoreError(err));
    } finally {
      running.current = false;
    }
  };
  usePrimaryAction("edit", () => void run());

  const upscale = async (factor: 2 | 4) => {
    if (store.getState().job || importing || !current) return;
    retry.current = { kind: "upscale", factor };
    setError(null);
    try {
      await actions.upscaleEdit(factor);
    } catch (err) {
      setError(api.asCoreError(err));
    }
  };
  const runAction = async (f: () => Promise<unknown>) => {
    retry.current = { kind: "action", f };
    setError(null);
    try {
      await f();
    } catch (err) {
      setError(api.asCoreError(err));
    }
  };

  // Ctrl/Cmd+Z / Shift+Z for the edit chain (not while typing).
  useEffect(() => {
    if (tab !== "edit") return;
    const onKey = (ev: KeyboardEvent) => {
      if (!(ev.ctrlKey || ev.metaKey) || ev.key.toLowerCase() !== "z") return;
      const t = ev.target as HTMLElement | null;
      if (
        t &&
        (t.tagName === "TEXTAREA" ||
          t.tagName === "INPUT" ||
          t.isContentEditable)
      )
        return;
      ev.preventDefault();
      if (editBusy(store.getState())) return;
      const s = store.getState().edit;
      dispatch({ type: "editGoto", index: s.index + (ev.shiftKey ? 1 : -1) });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab, dispatch, store]);

  const needsEditModel = mode === "instruction" && !autoEdit;
  const text =
    mode === "instruction"
      ? e.instruction
      : fixing
        ? e.fixPrompt
        : extending
          ? e.extendPrompt
          : e.restylePrompt;
  const ready = !!current && !!model && !importing && !needsEditModel;
  const canRun =
    ready &&
    (fixing
      ? painted
      : extending
        ? !!canvas
        : text.trim().length > 0 || !!e.styleId);
  // An upscale step has nothing to redo; the original has no step before it.
  // Fix details redoes the step with the spot painted for it. Not while edits run or wait:
  // redoing a step drops the steps after it, which could be their results.
  // Extend redoes from the step before, which is smaller than the shown result.
  const prevImg =
    e.index > 0 ? images[e.chain[e.index - 1]?.imageId] : undefined;
  const canExtendAgain =
    extending &&
    ready &&
    !!prevImg &&
    !!extendCanvas(prevImg.width, prevImg.height, e.extendTo, e.extendSide, ui);
  const canTryAgain =
    (fixing
      ? ready && !!masks.current.get(current?.id ?? "")
      : extending
        ? canExtendAgain
        : canRun) &&
    !locked &&
    e.index > 0 &&
    !!node?.meta &&
    node.meta.kind !== "upscaled";
  useShortcuts("edit", {
    tryAgain: canTryAgain ? () => void run(true) : undefined,
    describe: current ? () => actions.sendToDescribe(current.id) : undefined,
  });
  // One-time notice the first time a picture from the computer is edited (RELEASE-SPEC §7).
  const rootId = e.chain[0]?.imageId;
  const rootImported = useAppState(
    (s) =>
      !!rootId &&
      s.results.find((r) => r.id === rootId)?.origin !== "generated",
  );
  const editNoticeSeen = useAppState(
    (s) => s.settings?.editNoticeSeen ?? true,
  );
  const [editNoticeClosed, setEditNoticeClosed] = useState(false);
  const showEditNotice = rootImported && !editNoticeSeen && !editNoticeClosed;
  const closeEditNotice = () => {
    setEditNoticeClosed(true);
    void api
      .getSettings()
      .then((s) => api.setSettings({ ...s, editNoticeSeen: true }))
      .then((s) => dispatch({ type: "setSettings", settings: s }))
      .catch(() => undefined);
  };
  const loras = useAppState((s) => s.loras);
  const addTriggerWords = useAppState(
    (s) => s.settings?.addTriggerWords ?? true,
  );
  const previewReq = useMemo(
    () =>
      current &&
      model &&
      outSize &&
      (text.trim() || e.styleId || fixing || extending)
        ? buildEditRequest(e, {
            mode,
            source: current,
            model,
            ui,
            maskImageId: null,
            size: outSize,
            loras,
            autoAdd: addTriggerWords,
          })
        : null,
    [
      e,
      mode,
      current,
      model,
      ui,
      outSize?.[0],
      outSize?.[1],
      loras,
      addTriggerWords,
      text,
    ],
  );

  return (
    <div className="grid h-full grid-cols-[minmax(360px,420px)_minmax(0,1fr)]">
      <aside
        aria-label="Edit settings"
        className="flex min-h-0 flex-col border-r border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900"
      >
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pt-4 pb-5">
          {showEditNotice && (
            <div
              role="note"
              className="flex items-start gap-3 rounded-xl border border-amber-200 bg-amber-50 px-3 py-2.5 text-xs text-amber-900 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-200"
            >
              <p className="flex-1">
                Only edit photos of people who have agreed to it. Making sexual
                or humiliating images of real people without their consent is a
                crime in many countries.
              </p>
              <Button size="sm" variant="ghost" onClick={closeEditNotice}>
                OK
              </Button>
            </div>
          )}
          <div>
            <Segmented
              stretch
              size="sm"
              ariaLabel="Edit mode"
              value={mode}
              onChange={(m) =>
                dispatch({ type: "patchEdit", patch: { mode: m } })
              }
              options={[
                {
                  value: "instruction" as EditMode,
                  label: "Describe a change",
                  title: "Tell it what to change; the rest stays",
                },
                {
                  value: "restyle" as EditMode,
                  label: "Restyle",
                  title: "Redraw the whole image in a new look",
                },
                {
                  value: "fix" as EditMode,
                  label: "Fix details",
                  title: "Redraw a small spot, like a face or hand, sharper",
                },
                {
                  value: "extend" as EditMode,
                  label: "Extend",
                  title:
                    "Make the picture wider or taller; the new edges are drawn to match",
                },
              ]}
            />
            <p className="mt-1.5 text-xs text-neutral-500">
              {e.mode == null
                ? autoEdit
                  ? autoEdit.isEditModel
                    ? "Picked automatically — you have an edit model."
                    : "Picked automatically — one of your models can edit."
                  : "Picked automatically — Restyle works with your Create model."
                : mode === "instruction"
                  ? "Say what should change. Everything else stays the same."
                  : fixing
                    ? "Paint over a small spot, like a face or hand. It's redrawn larger, then blended back in."
                    : extending
                      ? "Pick a new shape. Pinhole adds space around your picture and draws what fits there."
                      : "Redraws the whole picture with your description."}
            </p>
          </div>

          {mode === "instruction" ? (
            needsEditModel ? (
              <div className="space-y-3">
                <div className="rounded-xl bg-neutral-50 p-3 text-sm dark:bg-neutral-800/50">
                  <div className="font-medium">
                    {noGpu
                      ? "Describing a change needs a graphics card"
                      : twoImages
                        ? "Combining two images needs another edit model"
                        : "Get the best edit model for your GPU"}
                  </div>
                  <p className="mt-0.5 text-xs text-neutral-500">
                    {noGpu
                      ? "Pinhole didn't find one it can use, and edit models are too big for the processor. Switch to Restyle — it works with the model you already have."
                      : twoImages
                        ? "Qwen Image Edit and FLUX.2 models can use a second image. Or remove the second image to edit with the model you have."
                        : "Edit models change just what you ask for. Or switch to Restyle — it works with the model you already have."}
                  </p>
                </div>
                <RecommendedCards
                  roles={twoImages ? ["edit"] : ["edit", "edit_alt"]}
                  families={twoImages ? TWO_IMAGE_PICKS : undefined}
                  compact
                />
              </div>
            ) : (
              <>
                <ModelPicker
                  models={edits}
                  value={editModelId}
                  onChange={(id) =>
                    dispatch({ type: "patchEdit", patch: { editModelId: id } })
                  }
                  label="Edit model"
                />
                {editFit && editFit !== "fits" && !noGpu && (
                  <RecommendedCards
                    roles={twoImages ? ["edit"] : ["edit", "edit_alt"]}
                    families={twoImages ? TWO_IMAGE_PICKS : undefined}
                    compact
                    offers="all"
                    heading={
                      <p className="text-xs text-neutral-500">
                        {editFit === "tight"
                          ? "This edit model is a tight fit for your graphics card: it runs slower and can run out of memory. Models that fit:"
                          : "This edit model is probably too big for your graphics card. Models that fit:"}
                      </p>
                    }
                  />
                )}
                <div>
                  <label
                    htmlFor="edit-instruction"
                    className="mb-1.5 block text-sm font-medium"
                  >
                    What should change?
                  </label>
                  <AutoTextarea
                    id="edit-instruction"
                    minRows={3}
                    maxRows={10}
                    value={e.instruction}
                    placeholder={
                      twoImages
                        ? "e.g. put the bottle from image 2 on the shelf in the background, same label and colors"
                        : "e.g. make it evening with warm street lights, or replace the mug with a water bottle"
                    }
                    onChange={(ev) =>
                      dispatch({
                        type: "patchEdit",
                        patch: { instruction: ev.target.value },
                      })
                    }
                  />
                </div>
                {(ui?.stayCloseShown ?? true) && (
                  <div>
                    <div className="mb-1 text-sm text-neutral-600 dark:text-neutral-400">
                      Stay close to original
                    </div>
                    <Slider
                      ariaLabel="Stay close to original"
                      value={e.stayClose ?? defaultStayClosePosition(ui)}
                      onChange={(v) =>
                        dispatch({ type: "patchEdit", patch: { stayClose: v } })
                      }
                      left="Loose"
                      right="Close"
                    />
                  </div>
                )}
              </>
            )
          ) : (
            <>
              {creates.length ? (
                <ModelPicker
                  models={creates}
                  value={restyleModelId}
                  onChange={(id) =>
                    dispatch({
                      type: "patchEdit",
                      patch: { restyleModelId: id },
                    })
                  }
                  label="Model"
                />
              ) : (
                <RecommendedCards roles={["realistic", "anime"]} compact />
              )}
              {extending && current && (
                <ExtendControls
                  to={e.extendTo}
                  side={e.extendSide}
                  width={current.width}
                  height={current.height}
                  ui={ui}
                  onChange={(patch) => dispatch({ type: "patchEdit", patch })}
                />
              )}
              <div>
                {extending ? (
                  <>
                    <label
                      htmlFor="edit-extend"
                      className="mb-1.5 block text-sm font-medium"
                    >
                      What's in the picture?{" "}
                      <span className="font-normal text-neutral-500">
                        (optional)
                      </span>
                    </label>
                    <AutoTextarea
                      id="edit-extend"
                      minRows={2}
                      maxRows={6}
                      value={e.extendPrompt}
                      placeholder="e.g. a sandy beach at sunset with palm trees"
                      onChange={(ev) =>
                        dispatch({
                          type: "patchEdit",
                          patch: { extendPrompt: ev.target.value },
                        })
                      }
                    />
                    <p className="mt-1 text-xs text-neutral-500">
                      Describe the whole scene, not just the new part.
                    </p>
                  </>
                ) : fixing ? (
                  <>
                    <label
                      htmlFor="edit-fix"
                      className="mb-1.5 block text-sm font-medium"
                    >
                      What is it?{" "}
                      <span className="font-normal text-neutral-500">
                        (optional)
                      </span>
                    </label>
                    <AutoTextarea
                      id="edit-fix"
                      minRows={2}
                      maxRows={6}
                      value={e.fixPrompt}
                      placeholder="e.g. a hand holding a cup, or a street sign"
                      onChange={(ev) =>
                        dispatch({
                          type: "patchEdit",
                          patch: { fixPrompt: ev.target.value },
                        })
                      }
                    />
                  </>
                ) : (
                  <>
                    <label
                      htmlFor="edit-restyle"
                      className="mb-1.5 block text-sm font-medium"
                    >
                      What should it look like?
                    </label>
                    <AutoTextarea
                      id="edit-restyle"
                      minRows={3}
                      maxRows={10}
                      value={e.restylePrompt}
                      placeholder="e.g. a watercolor painting of the same scene"
                      onChange={(ev) =>
                        dispatch({
                          type: "patchEdit",
                          patch: { restylePrompt: ev.target.value },
                        })
                      }
                    />
                  </>
                )}
              </div>
              {!extending && (
                <div>
                  <div className="mb-1.5 text-sm text-neutral-600 dark:text-neutral-400">
                    How much to change
                  </div>
                  <Segmented
                    stretch
                    ariaLabel="How much to change"
                    value={e.change}
                    onChange={(v) =>
                      dispatch({ type: "patchEdit", patch: { change: v } })
                    }
                    options={[
                      { value: "subtle" as ChangeAmount, label: "Subtle" },
                      { value: "medium" as ChangeAmount, label: "Medium" },
                      { value: "strong" as ChangeAmount, label: "Strong" },
                    ]}
                  />
                </div>
              )}
            </>
          )}

          {!needsEditModel && <AddonChips model={model} target="edit" />}

          {mode === "instruction" && current && (
            <div>
              {secondPicker.input}
              {second ? (
                <div className="flex items-center gap-3 rounded-xl border border-neutral-200 p-2 dark:border-neutral-800">
                  <img
                    src={second.url}
                    alt="Image 2"
                    className="h-12 w-12 shrink-0 rounded-md object-cover"
                    draggable={false}
                  />
                  <p className="min-w-0 flex-1 text-xs text-neutral-500">
                    <span className="font-medium text-neutral-700 dark:text-neutral-300">
                      Image 2.
                    </span>{" "}
                    Call the picture you're editing “image 1” and this one
                    “image 2”.
                  </p>
                  <IconButton
                    label="Remove image 2"
                    size="sm"
                    variant="ghost"
                    disabled={myJob}
                    onClick={() =>
                      dispatch({ type: "editSetSecond", ref: null })
                    }
                  >
                    <Trash className="h-3.5 w-3.5" />
                  </IconButton>
                </div>
              ) : (
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={locked || importing}
                  onClick={secondPicker.open}
                  title="Use something from another picture, like an object or a logo"
                >
                  <ImagePlus className="h-3.5 w-3.5" /> Add another image
                </Button>
              )}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <StylePicker
              value={e.styleId}
              onChange={(id) =>
                dispatch({ type: "patchEdit", patch: { styleId: id } })
              }
              familyId={model?.familyId}
              familyLabel={model?.familyLabel}
            />
            {e.styleId && mode === "instruction" && (
              <span className="text-xs text-neutral-500">
                “make it look like: …”
              </span>
            )}
          </div>

          {!twoImages && !extending && (
            <div className="rounded-xl border border-neutral-200 p-3 dark:border-neutral-800">
              {fixing ? (
                <div>
                  <div className="inline-flex items-center gap-1.5 text-sm font-medium">
                    <Brush className="h-3.5 w-3.5" /> Spot to fix
                  </div>
                  <p className="mt-0.5 text-xs text-neutral-500">
                    {painted
                      ? "Paint a little past the edges so it blends in."
                      : "Paint over the face, hand or detail to redraw."}
                  </p>
                </div>
              ) : (
                <Toggle
                  checked={maskOn}
                  onChange={setMaskOn}
                  label={
                    <span className="inline-flex items-center gap-1.5 font-medium">
                      <Brush className="h-3.5 w-3.5" /> Only change here
                    </span>
                  }
                  hint={
                    maskOn
                      ? "Paint over the part of the image that may change."
                      : "Optional: paint the area to change."
                  }
                />
              )}
              {maskOn && (
                <div className="mt-3 space-y-2.5">
                  <div className="flex items-center gap-2">
                    <Segmented
                      size="sm"
                      ariaLabel="Brush or eraser"
                      value={erase ? "erase" : "paint"}
                      onChange={(v) => setErase(v === "erase")}
                      options={[
                        {
                          value: "paint",
                          label: (
                            <>
                              <Brush className="h-3 w-3" /> Paint
                            </>
                          ),
                        },
                        {
                          value: "erase",
                          label: (
                            <>
                              <Eraser className="h-3 w-3" /> Erase
                            </>
                          ),
                        },
                      ]}
                    />
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => mask.current?.clear()}
                      disabled={!painted}
                    >
                      <Trash className="h-3.5 w-3.5" /> Clear
                    </Button>
                  </div>
                  <Slider
                    ariaLabel="Brush size"
                    min={6}
                    max={160}
                    step={1}
                    value={brush}
                    onChange={setBrush}
                    left="Brush"
                    right={<span className="tabular-nums">{brush}px</span>}
                  />
                </div>
              )}
            </div>
          )}

          <section className="rounded-xl border border-neutral-200 dark:border-neutral-800">
            <button
              type="button"
              aria-expanded={moreOpen}
              onClick={() => setMoreOpen((o) => !o)}
              className={cx(
                "flex w-full items-center gap-2 rounded-xl px-3 py-2.5 text-left text-sm font-medium",
                focusRing,
              )}
            >
              <SlidersHorizontal className="h-4 w-4 text-neutral-500" />{" "}
              Fine-tune
              <ChevronDown
                className={cx(
                  "ml-auto h-4 w-4 text-neutral-400 transition-transform",
                  moreOpen && "rotate-180",
                )}
              />
            </button>
            {moreOpen && (
              <div className="space-y-3 border-t border-neutral-200 px-3 pt-3 pb-4 dark:border-neutral-800">
                <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-2 text-sm">
                  <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
                    Quality
                  </span>
                  <Segmented
                    size="sm"
                    ariaLabel="Quality"
                    value={e.quality}
                    onChange={(q) =>
                      dispatch({ type: "patchEdit", patch: { quality: q } })
                    }
                    options={(["fast", "balanced", "best"] as Quality[]).map(
                      (q, i) => ({
                        value: q,
                        label: q[0].toUpperCase() + q.slice(1),
                        title: ui ? `${ui.qualitySteps[i]} steps` : undefined,
                      }),
                    )}
                  />
                  {!fixing && !extending && (
                    <>
                      <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
                        Output size
                      </span>
                      <div className="flex items-center gap-2">
                        <Segmented
                          size="sm"
                          ariaLabel="Output size"
                          value={size}
                          onChange={setSize}
                          options={[
                            {
                              value: "smaller" as SizeChoice,
                              label: "Smaller",
                            },
                            { value: "normal" as SizeChoice, label: "Normal" },
                            { value: "larger" as SizeChoice, label: "Larger" },
                          ]}
                        />
                        {outSize && (
                          <span className="text-xs text-neutral-500 tabular-nums">
                            {outSize[0]}×{outSize[1]}
                          </span>
                        )}
                      </div>
                    </>
                  )}
                  <label
                    htmlFor="edit-seed"
                    className="text-xs font-medium text-neutral-600 dark:text-neutral-400"
                  >
                    Seed
                  </label>
                  <input
                    id="edit-seed"
                    type="number"
                    className={cx(inputClass, "h-8 py-0")}
                    placeholder="Random"
                    value={e.seed ?? ""}
                    onChange={(ev) => {
                      const n = Number.parseInt(ev.target.value, 10);
                      dispatch({
                        type: "patchEdit",
                        patch: {
                          seed: Number.isFinite(n) && n >= 0 ? n : null,
                        },
                      });
                    }}
                  />
                  <LoraSection model={model} target="edit" />
                  <PromptPreview
                    req={previewReq}
                    empty={
                      mode === "instruction"
                        ? "Say what should change to see exactly what is sent."
                        : "Describe how it should look to see exactly what is sent."
                    }
                  />
                </div>
                <p className="text-[11px] text-neutral-400">
                  {fixing
                    ? "The picture keeps its size; only the painted spot changes. "
                    : extending
                      ? "The picture keeps its detail; the new space is drawn at the model's size and scaled to fit. "
                      : "Size keeps your image’s shape. "}
                  {ui
                    ? `${ui.label} defaults are used for everything else.`
                    : ""}
                </p>
              </div>
            )}
          </section>
        </div>

        <div className="shrink-0 space-y-2 border-t border-neutral-200 px-5 py-4 dark:border-neutral-800">
          {myJob && (
            <LiveJobProgress
              kinds={EDIT_JOBS}
              cancelling={cancelling}
              onCancel={async () => {
                setCancelling(true);
                await actions.cancel();
                setCancelling(false);
              }}
            />
          )}
          <div className="flex gap-2">
            <Button
              variant="primary"
              size="lg"
              className="min-w-0 flex-1"
              disabled={!canRun}
              onClick={() => void run()}
            >
              {queues ? (
                <ListPlus className="h-4 w-4" />
              ) : (
                <WandSparkles className="h-4 w-4" />
              )}
              {queues
                ? "Add to queue"
                : mode === "instruction"
                  ? "Apply edit"
                  : fixing
                    ? "Fix details"
                    : extending
                      ? "Extend"
                      : "Restyle"}
              <span className="ml-1 inline-flex gap-0.5 opacity-70">
                <Kbd>{modKey}</Kbd>
                <Kbd>Enter</Kbd>
              </span>
            </Button>
            <QueueButton />
          </div>
          {!current && !myJob && (
            <p className="text-center text-xs text-neutral-500">
              Add an image to start.
            </p>
          )}
          {job && !myJob && (
            <p className="text-center text-xs text-neutral-500">
              Busy creating. Edits wait for it to finish.
            </p>
          )}
          {error && (
            <ErrorWithFix
              error={error}
              onDismiss={() => setError(null)}
              onRetry={() => {
                const r = retry.current;
                if (r.kind === "upscale") void upscale(r.factor);
                else if (r.kind === "action") void runAction(r.f);
                else void run(r.again);
              }}
            />
          )}
        </div>
      </aside>

      <DropTarget
        onFile={(f) => void load(f)}
        className="flex min-h-0 min-w-0 flex-col bg-neutral-100 dark:bg-neutral-950"
        label="Drop to edit this image"
      >
        {picker.input}
        {!current ? (
          <div className="flex min-h-0 flex-1 p-6">
            <DropZone
              onFile={(f) => void load(f)}
              title="Add an image to edit"
              busy={importing}
            >
              {importing && <Spinner className="mt-3 h-4 w-4" />}
            </DropZone>
          </div>
        ) : (
          <>
            <div className="flex shrink-0 items-center gap-1.5 border-b border-neutral-200 bg-white/60 px-4 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
              <IconButton
                label="Undo"
                disabled={e.index === 0 || locked}
                onClick={() =>
                  dispatch({ type: "editGoto", index: e.index - 1 })
                }
              >
                <Undo2 className="h-4 w-4" />
              </IconButton>
              <IconButton
                label="Redo"
                disabled={e.index >= e.chain.length - 1 || locked}
                onClick={() =>
                  dispatch({ type: "editGoto", index: e.index + 1 })
                }
              >
                <Redo2 className="h-4 w-4" />
              </IconButton>
              <IconButton
                label="Delete this edit"
                disabled={e.index === 0 || job || locked}
                onClick={() => dispatch({ type: "editDelete", index: e.index })}
              >
                <Trash2 className="h-4 w-4" />
              </IconButton>
              <span className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />
              <Button
                size="sm"
                variant="ghost"
                disabled={!canTryAgain}
                onClick={() => void run(true)}
                title={
                  e.index < e.chain.length - 1
                    ? "Make this edit again from the step before, with a new seed. Replaces it and the edits after it."
                    : "Make this edit again from the step before, with a new seed. Replaces it."
                }
                aria-label="Try again"
              >
                <RefreshCw className="h-3.5 w-3.5" />{" "}
                <span className="hidden xl:inline">Try again</span>
              </Button>
              <Button
                size="sm"
                variant={compare && before ? "secondary" : "ghost"}
                disabled={!before && e.index === 0}
                aria-pressed={compare && !!before}
                onClick={() => setCompare((c) => !c)}
              >
                <Columns2 className="h-3.5 w-3.5" /> Compare
              </Button>
              {compare && e.index > 1 && (
                <Segmented
                  size="sm"
                  ariaLabel="Compare with"
                  value={compareWith}
                  onChange={setCompareWith}
                  options={[
                    { value: "previous", label: "Previous" },
                    { value: "original", label: "Original" },
                  ]}
                />
              )}
              <div className="ml-auto flex items-center gap-1.5">
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={locked}
                  onClick={picker.open}
                  title="Edit a different image"
                  aria-label="New image"
                >
                  <ImagePlus className="h-3.5 w-3.5" />{" "}
                  <span className="hidden xl:inline">New image</span>
                </Button>
                <UpscaleMenu
                  size="sm"
                  width={current.width}
                  height={current.height}
                  disabled={job || importing}
                  onPick={(f) => void upscale(f)}
                />
                <SaveButton
                  tab="edit"
                  size="sm"
                  id={current.id}
                  seed={node?.meta?.seed ?? null}
                  run={runAction}
                />
                <IconButton
                  label="Copy image"
                  size="sm"
                  variant="secondary"
                  onClick={() =>
                    void actions
                      .copyImage(current.id)
                      .catch((err) => setError(api.asCoreError(err)))
                  }
                >
                  <Copy className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton
                  label="Describe this image"
                  size="sm"
                  variant="secondary"
                  onClick={() => actions.sendToDescribe(current.id)}
                >
                  <ScanText className="h-3.5 w-3.5" />
                </IconButton>
              </div>
            </div>

            <Stage
              current={current}
              before={compare ? before : undefined}
              beforeLabel={
                compareWith === "original"
                  ? "Original"
                  : (prevNode?.label ?? "Before")
              }
              afterLabel={node?.label ?? "After"}
              maskOn={maskOn && !twoImages && !(compare && before)}
              canvas={canvas && !(compare && before) ? canvas : null}
              maskRef={mask}
              brush={brush}
              erase={erase}
              onPainted={setPainted}
            />
            {node?.meta && (
              <p className="-mt-3 shrink-0 pb-3 text-center text-xs text-neutral-500 tabular-nums">
                {settingsSummary(node.meta)}
              </p>
            )}
            {import.meta.env.DEV && node && (
              <div className="-mt-2 shrink-0 pb-2">
                <CheckReadings id={node.imageId} />
              </div>
            )}

            <div className="shrink-0 border-t border-neutral-200 bg-white/60 px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900/40">
              <ol
                className="flex items-center gap-1.5 overflow-x-auto pb-1"
                aria-label="Edit history"
              >
                {e.chain.map((n, i) => {
                  const img = images[n.imageId];
                  return (
                    <li
                      key={n.imageId}
                      className="flex shrink-0 items-center gap-1.5"
                    >
                      {i > 0 && (
                        <ArrowRight
                          className="h-3.5 w-3.5 text-neutral-400"
                          aria-hidden
                        />
                      )}
                      <button
                        type="button"
                        aria-current={i === e.index ? "step" : undefined}
                        disabled={locked && i !== e.index}
                        onClick={() => dispatch({ type: "editGoto", index: i })}
                        className={cx(
                          "group flex flex-col items-center gap-1 rounded-lg p-1 disabled:cursor-not-allowed disabled:opacity-50",
                          focusRing,
                          i === e.index
                            ? "bg-amber-50 dark:bg-amber-500/10"
                            : "hover:bg-neutral-100 dark:hover:bg-neutral-800",
                        )}
                      >
                        {img && (
                          <img
                            src={img.url}
                            alt=""
                            className={cx(
                              "h-14 w-14 rounded-md object-cover ring-2",
                              i === e.index
                                ? "ring-amber-500"
                                : "ring-transparent",
                            )}
                            draggable={false}
                          />
                        )}
                        <span
                          className={cx(
                            "text-[11px]",
                            i === e.index
                              ? "font-medium text-amber-900 dark:text-amber-200"
                              : "text-neutral-500",
                          )}
                        >
                          {n.label}
                        </span>
                      </button>
                    </li>
                  );
                })}
              </ol>
            </div>
          </>
        )}
      </DropTarget>
    </div>
  );
}

function Stage({
  current,
  before,
  beforeLabel,
  afterLabel,
  maskOn,
  maskRef,
  brush,
  erase,
  onPainted,
  canvas,
}: {
  current: { id: string; url: string; width: number; height: number };
  before?: { id: string; url: string; width: number; height: number };
  beforeLabel: string;
  afterLabel: string;
  maskOn: boolean;
  maskRef: RefObject<MaskHandle | null>;
  brush: number;
  erase: boolean;
  onPainted: (b: boolean) => void;
  /** Extend's new canvas (source pixels): shown as a dashed frame around the picture. */
  canvas: ExtendCanvas | null;
}) {
  const container = useRef<HTMLDivElement>(null);
  const box = useFitBox(
    container,
    canvas?.width ?? current.width,
    canvas?.height ?? current.height,
  );
  // With a canvas, `box` is the canvas; the picture sits inside it.
  const pic = canvas
    ? {
        left: (canvas.left / canvas.width) * box.width,
        top: (canvas.top / canvas.height) * box.height,
        width: (current.width / canvas.width) * box.width,
        height: (current.height / canvas.height) * box.height,
      }
    : { left: 0, top: 0, width: box.width, height: box.height };
  const [viewing, setViewing] = useState(false);
  useShortcuts("edit", {
    fullscreen: maskOn ? undefined : () => setViewing(true),
  });
  return (
    <div
      ref={container}
      className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-6"
    >
      {box.width > 0 && before && (
        <CompareView
          before={before}
          after={current}
          width={box.width}
          height={box.height}
          beforeLabel={beforeLabel}
          afterLabel={afterLabel}
        />
      )}
      {canvas && !before && box.width > 0 && (
        <div
          aria-label="New space"
          className="absolute rounded-lg border-2 border-dashed border-amber-500/80 bg-amber-500/10"
          style={{ width: box.width, height: box.height }}
        />
      )}
      {/* Kept mounted while comparing so a painted mask isn't lost. */}
      <div
        hidden={!!before || box.width === 0}
        className={cx(
          "relative overflow-hidden shadow-lg ring-1 ring-black/5 dark:ring-white/10",
          canvas ? "rounded-sm" : "rounded-lg",
        )}
        style={{
          width: pic.width,
          height: pic.height,
          transform: canvas
            ? `translate(${pic.left + pic.width / 2 - box.width / 2}px, ${pic.top + pic.height / 2 - box.height / 2}px)`
            : undefined,
        }}
      >
        <img
          src={current.url}
          alt={`Image being edited (${afterLabel})`}
          className="absolute inset-0 h-full w-full"
          draggable={false}
        />
        {!maskOn && (
          <IconButton
            label="View full screen"
            size="sm"
            className="absolute top-2 right-2 z-10 bg-black/40 text-white hover:bg-black/60"
            onClick={() => setViewing(true)}
          >
            <Maximize2 className="h-4 w-4" />
          </IconButton>
        )}
        <MaskCanvas
          ref={maskRef}
          width={current.width}
          height={current.height}
          displayWidth={pic.width}
          brush={brush}
          erase={erase}
          active={maskOn}
          onPaintedChange={onPainted}
        />
      </div>
      {viewing && (
        <ImageViewer
          images={[
            {
              url: current.url,
              width: current.width,
              height: current.height,
              alt: `Image being edited (${afterLabel})`,
            },
          ]}
          index={0}
          onIndex={() => {}}
          onClose={() => setViewing(false)}
        />
      )}
    </div>
  );
}
