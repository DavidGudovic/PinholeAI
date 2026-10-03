// Edit (img2img + instruction editing), SPEC §5.2.
// The image lives in the Rust session (RAM); the edit chain is an in-memory undo stack.
import { useEffect, useMemo, useRef, useState } from "react";
import { DropTarget, DropZone, useFilePicker } from "../../components/ImageDrop";
import { CheckReadings } from "../../components/CheckReadings";
import { SessionChoices, useSessionPictures } from "../../components/SessionPictures";
import { useUpscaler } from "../../components/UpscalerChoice";
import { StylePicker } from "../../components/StylePicker";
import { Spinner } from "../../components/ui";
import { useHardware } from "../models/lib/hooks";
import { isCpuOnly } from "../models/lib/words";
import * as api from "../../lib/api";
import { sizeMultiple } from "../../lib/paste/map";
import type { CoreError } from "../../lib/types";
import { useShortcuts } from "../../lib/shortcuts";
import { useActions, usePrimaryAction } from "../../lib/state/AppProvider";
import { useFamilyUi, useModel } from "../../lib/state/hooks";
import {
  createModels,
  editBusy,
  editModels,
  initialEdit,
  isEditJob,
  willQueue,
  type EditMode,
} from "../../lib/state/model";
import {
  buildEditRequest,
  editOutputSize,
  extendCanvas,
  settingsSummary,
  type EditSizeChoice,
} from "../../lib/state/request";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import { AddonChips } from "../create/AddonChips";
import { ApplyBar } from "./ApplyBar";
import { EditFineTune } from "./EditFineTune";
import { EditHistory } from "./EditHistory";
import { EditModePicker } from "./EditModePicker";
import { EditNotice } from "./EditNotice";
import { EditToolbar } from "./EditToolbar";
import { InstructionFields } from "./InstructionFields";
import type { MaskHandle } from "./MaskCanvas";
import { MaskControls } from "./MaskControls";
import { RedrawFields } from "./RedrawFields";
import { SecondImage } from "./SecondImage";
import { Stage } from "./Stage";

type SizeChoice = EditSizeChoice;

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
  // Side by side: the second image an edit combined, next to the result.
  const [sideBySide, setSideBySide] = useState(false);
  const [compareWith, setCompareWith] = useState<"previous" | "original">(
    "previous",
  );
  const [size, setSize] = useState<SizeChoice>("normal");
  const [moreOpen, setMoreOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const mask = useRef<MaskHandle>(null);
  // The mask each edit was made with (by result image), so "Try again" repaints the same area.
  const masks = useRef(new Map<string, Blob | null>());
  // Steps made by Add detail (Fix details with nothing painted): Try again redoes the faces.
  const wholeDetail = useRef(new Set<string>());
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
  const nodeSecond = node?.secondImageId ? images[node.secondImageId] : undefined;
  const showSide = sideBySide && !!nodeSecond;
  const comparing = compare && !!before && !showSide;
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
      setSideBySide(false);
    } catch (err) {
      setError(api.asCoreError(err));
    } finally {
      setImporting(false);
    }
  };
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
  // This session's pictures: another one to edit (not the one shown), or image 2 (not image 1).
  const sessionPics = useSessionPictures([node?.imageId]);
  const pickSession = (id: string) => {
    if (editBusy(store.getState())) return;
    setError(null);
    actions.sendToEdit(id);
  };
  const pickSecond = (id: string) => {
    const ref = store.getState().images[id];
    if (!ref || editBusy(store.getState())) return;
    setError(null);
    dispatch({ type: "editSetSecond", ref });
  };

  // A new current image means a new mask.
  useEffect(() => {
    mask.current?.clear();
  }, [node?.imageId]);
  // Another image (or Reset) starts a new history.
  const originalId = e.chain[0]?.imageId;
  useEffect(() => {
    masks.current.clear();
    wholeDetail.current.clear();
    setCompare(false);
    setSideBySide(false);
    setError(null);
  }, [originalId]);

  // Set before the first await (the mask export), so a second click or Ctrl+Enter
  // during the export doesn't get as far as the job and report "still working".
  const running = useRef(false);
  // What "Retry" on an error repeats (the edit, Try again, upscale or save that failed).
  // Set when it fails, since a queued one can fail after others were pressed.
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
    setError(null);
    try {
      const m = again
        ? (masks.current.get(current.id) ?? null)
        : maskOn && painted && !twoImages
          ? ((await mask.current?.exportPng()) ?? null)
          : null;
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
        // Only Try again names the step: a normal edit uses the step shown when it runs, so
        // one queued behind another is added after its result instead of replacing it.
        from: again ? from : undefined,
        newSeed: again,
      });
      running.current = false;
      await done;
      const now = store.getState().edit;
      const made = now.chain[now.index];
      if (now.index === from + 1 && made && made.imageId !== current.id) {
        masks.current.set(made.imageId, m);
        if (fixing && !m) wholeDetail.current.add(made.imageId);
        setCompare(true);
        setCompareWith("previous");
        setSideBySide(false);
      }
    } catch (err) {
      retry.current = { kind: "edit", again };
      setError(api.asCoreError(err));
    } finally {
      running.current = false;
    }
  };
  usePrimaryAction("edit", () => void run());

  const upscale = async (factor: 2 | 4) => {
    if (importing || !current) return;
    setError(null);
    try {
      await actions.upscaleEdit(factor);
    } catch (err) {
      retry.current = { kind: "upscale", factor };
      setError(api.asCoreError(err));
    }
  };
  const runAction = async (f: () => Promise<unknown>) => {
    setError(null);
    try {
      await f();
    } catch (err) {
      retry.current = { kind: "action", f };
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
          t.tagName === "SELECT" ||
          t.isContentEditable)
      )
        return;
      // Not behind a dialog, sheet or the viewer.
      if (document.querySelector('[role="dialog"]')) return;
      ev.preventDefault();
      if (editBusy(store.getState())) return;
      const s = store.getState().edit;
      dispatch({ type: "editGoto", index: s.index + (ev.shiftKey ? 1 : -1) });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab, dispatch, store]);

  const upscaler = useUpscaler();
  // Shown on the closed Fine-tune header, as in Create, so a fixed seed or size isn't hidden.
  const fineTuneChanged =
    (e.seed != null ? 1 : 0) +
    (e.loras.length ? 1 : 0) +
    (e.quality !== initialEdit().quality ? 1 : 0) +
    (size !== "normal" && !fixing && !extending ? 1 : 0);

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
    (fixing ||
      (extending ? !!canvas : text.trim().length > 0 || !!e.styleId));
  // An upscale step has nothing to redo; the original has no step before it.
  // Fix details redoes the step with the spot painted for it, or the faces again
  // after Add detail. Not while edits run or wait:
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
      ? ready &&
        (!!masks.current.get(current?.id ?? "") ||
          wholeDetail.current.has(current?.id ?? ""))
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
          <EditNotice rootId={e.chain[0]?.imageId} />
          <EditModePicker
            e={e}
            mode={mode}
            autoEdit={autoEdit}
            dispatch={dispatch}
          />

          {mode === "instruction" ? (
            <InstructionFields
              e={e}
              ui={ui}
              noGpu={noGpu}
              twoImages={twoImages}
              needsEditModel={needsEditModel}
              edits={edits}
              editModelId={editModelId}
              editFit={editFit}
              dispatch={dispatch}
            />
          ) : (
            <RedrawFields
              e={e}
              creates={creates}
              restyleModelId={restyleModelId}
              fixing={fixing}
              extending={extending}
              current={current}
              ui={ui}
              dispatch={dispatch}
            />
          )}

          {!needsEditModel && <AddonChips model={model} target="edit" />}

          {mode === "instruction" && current && (
            <SecondImage
              second={second}
              pickerInput={secondPicker.input}
              onPick={secondPicker.open}
              sessionPictures={sessionPics}
              onPickSession={(p) => pickSecond(p.id)}
              myJob={myJob}
              locked={locked}
              importing={importing}
              dispatch={dispatch}
            />
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
            <MaskControls
              fixing={fixing}
              maskOn={maskOn}
              setMaskOn={setMaskOn}
              painted={painted}
              erase={erase}
              setErase={setErase}
              brush={brush}
              setBrush={setBrush}
              onClear={() => mask.current?.clear()}
            />
          )}

          <EditFineTune
            e={e}
            mode={mode}
            fixing={fixing}
            extending={extending}
            painted={painted}
            model={model}
            ui={ui}
            moreOpen={moreOpen}
            setMoreOpen={setMoreOpen}
            fineTuneChanged={fineTuneChanged}
            size={size}
            setSize={setSize}
            outSize={outSize}
            upscaler={upscaler}
            previewReq={previewReq}
            dispatch={dispatch}
          />
        </div>

        <ApplyBar
          mode={mode}
          fixing={fixing}
          extending={extending}
          painted={painted}
          hasImage={!!current}
          job={job}
          myJob={myJob}
          queues={queues}
          canRun={canRun}
          onRun={() => void run()}
          cancelling={cancelling}
          onCancel={async () => {
            setCancelling(true);
            await actions.cancel();
            setCancelling(false);
          }}
          error={error}
          onDismiss={() => setError(null)}
          onRetry={() => {
            const r = retry.current;
            if (r.kind === "upscale") void upscale(r.factor);
            else if (r.kind === "action") void runAction(r.f);
            else void run(r.again);
          }}
        />
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
              <SessionChoices
                pictures={sessionPics}
                title="Edit this picture"
                disabled={importing}
                onPick={(p) => pickSession(p.id)}
              />
            </DropZone>
          </div>
        ) : (
          <>
            <EditToolbar
              e={e}
              current={current}
              node={node}
              before={before}
              locked={locked}
              job={job}
              importing={importing}
              canTryAgain={canTryAgain}
              onTryAgain={() => void run(true)}
              compare={comparing}
              setCompare={(v) => {
                setCompare(v);
                setSideBySide(false);
              }}
              canSideBySide={!!nodeSecond}
              sideBySide={showSide}
              setSideBySide={(v) => {
                setSideBySide(v);
                setCompare(false);
              }}
              compareWith={compareWith}
              setCompareWith={setCompareWith}
              onPickImage={picker.open}
              sessionPictures={sessionPics}
              onPickSession={(p) => pickSession(p.id)}
              onUpscale={(f) => void upscale(f)}
              runAction={runAction}
              onCopy={() =>
                void actions
                  .copyImage(current.id)
                  .catch((err) => setError(api.asCoreError(err)))
              }
              onDescribe={() => actions.sendToDescribe(current.id)}
              dispatch={dispatch}
            />

            <Stage
              current={current}
              before={comparing ? before : undefined}
              pair={
                showSide && nodeSecond
                  ? {
                      first: { ...nodeSecond, label: "Image 2" },
                      second: { ...current, label: node?.label ?? "Result" },
                    }
                  : undefined
              }
              beforeLabel={
                compareWith === "original"
                  ? "Original"
                  : (prevNode?.label ?? "Before")
              }
              afterLabel={node?.label ?? "After"}
              maskOn={maskOn && !twoImages && !comparing && !showSide}
              canvas={canvas && !comparing && !showSide ? canvas : null}
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

            <EditHistory
              e={e}
              images={images}
              locked={locked}
              dispatch={dispatch}
            />
          </>
        )}
      </DropTarget>
    </div>
  );
}
