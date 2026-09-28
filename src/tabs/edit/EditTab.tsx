// Edit (img2img + instruction editing), SPEC §5.2.
// The image lives in the Rust session (RAM); the edit chain is an in-memory undo stack.
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { ArrowRight, Brush, ChevronDown, Columns2, Copy, Eraser, ImagePlus, Redo2, Save, ScanText, SlidersHorizontal, Trash, Trash2, Undo2, WandSparkles } from "lucide-react";
import { DropTarget, DropZone, useFilePicker, useImagePaste } from "../../components/ImageDrop";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { JobProgress } from "../../components/JobProgress";
import { ModelPicker } from "../../components/ModelPicker";
import { StylePicker } from "../../components/StylePicker";
import { AutoTextarea, Button, IconButton, Kbd, Segmented, Slider, Spinner, Toggle, cx, focusRing, inputClass } from "../../components/ui";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { useHardware } from "../models/lib/hooks";
import { isCpuOnly } from "../models/lib/words";
import * as api from "../../lib/api";
import { defaultStickPosition, sizeMultiple } from "../../lib/paste/map";
import type { CoreError, Quality } from "../../lib/types";
import { useActions, usePrimaryAction } from "../../lib/state/AppProvider";
import { useFamilyUi, useModel } from "../../lib/state/hooks";
import { createModels, editModels, type ChangeAmount, type EditMode } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import { fitEditSize, settingsSummary } from "../../lib/state/request";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import { CompareView } from "./CompareView";
import { MaskCanvas, type MaskHandle } from "./MaskCanvas";
import { useFitBox } from "./useFitBox";

type SizeChoice = "smaller" | "normal" | "larger";
const SIZE_PIXELS: Record<SizeChoice, number> = { smaller: 640 * 640, normal: 1024 * 1024, larger: 1280 * 1280 };

export function EditTab() {
  const tab = useAppState((s) => s.tab);
  const e = useAppState((s) => s.edit);
  const images = useAppState((s) => s.images);
  const models = useAppState((s) => s.models);
  const createModelId = useAppState((s) => s.create.modelId);
  const job = useAppState((s) => s.job);
  const dispatch = useDispatch();
  const store = useStore();
  const actions = useActions();

  const [error, setError] = useState<CoreError | null>(null);
  const [importing, setImporting] = useState(false);
  const [maskOn, setMaskOn] = useState(false);
  const [brush, setBrush] = useState(40);
  const [erase, setErase] = useState(false);
  const [painted, setPainted] = useState(false);
  const [compare, setCompare] = useState(false);
  const [compareWith, setCompareWith] = useState<"previous" | "original">("previous");
  const [size, setSize] = useState<SizeChoice>("normal");
  const [moreOpen, setMoreOpen] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const mask = useRef<MaskHandle>(null);
  const hw = useHardware();
  const noGpu = !!hw?.detected && isCpuOnly(hw);

  const edits = useMemo(() => editModels(models), [models]);
  const creates = useMemo(() => createModels(models), [models]);
  // `models` is the real dependency: autoEditModel() reads the current list from the store.
  const autoEdit = useMemo(() => (models ? actions.autoEditModel() : null), [actions, models]);
  const mode: EditMode = e.mode ?? (autoEdit ? "instruction" : "restyle");
  const editModelId = e.editModelId ?? autoEdit?.id ?? null;
  const restyleModelId = e.restyleModelId ?? createModelId ?? creates[0]?.id ?? null;
  const model = useModel(mode === "instruction" ? editModelId : restyleModelId);
  const ui = useFamilyUi(model?.familyId);

  const node = e.chain[e.index] ?? null;
  const current = node ? images[node.imageId] : undefined;
  const prevNode = compareWith === "original" ? e.chain[0] : e.chain[e.index - 1];
  const before = e.index > 0 && prevNode ? images[prevNode.imageId] : undefined;
  const outSize = current ? fitEditSize(current.width, current.height, SIZE_PIXELS[size], sizeMultiple(model?.familyId)) : null;
  const myJob = job?.kind === "edit" ? job : null;

  const load = async (f: File) => {
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

  // A new current image means a new mask.
  useEffect(() => {
    mask.current?.clear();
  }, [node?.imageId]);

  const run = async () => {
    if (store.getState().job || !current || !model) return;
    setError(null);
    try {
      const m = maskOn && painted ? await mask.current?.exportPng() : null;
      const startIndex = store.getState().edit.index;
      await actions.runEdit({ mode, model, mask: m ?? null, size: outSize! });
      if (store.getState().edit.index > startIndex) {
        setCompare(true);
        setCompareWith("previous");
      }
    } catch (err) {
      setError(api.asCoreError(err));
    }
  };
  usePrimaryAction("edit", () => void run());

  // Ctrl/Cmd+Z / Shift+Z for the edit chain (not while typing).
  useEffect(() => {
    if (tab !== "edit") return;
    const onKey = (ev: KeyboardEvent) => {
      if (!(ev.ctrlKey || ev.metaKey) || ev.key.toLowerCase() !== "z") return;
      const t = ev.target as HTMLElement | null;
      if (t && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.isContentEditable)) return;
      ev.preventDefault();
      const s = store.getState().edit;
      dispatch({ type: "editGoto", index: s.index + (ev.shiftKey ? 1 : -1) });
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tab, dispatch, store]);

  const needsEditModel = mode === "instruction" && !autoEdit;
  const text = mode === "instruction" ? e.instruction : e.restylePrompt;
  const canRun = !!current && !!model && !job && (text.trim().length > 0 || !!e.styleId) && !needsEditModel;

  return (
    <div className="grid h-full grid-cols-[minmax(360px,420px)_minmax(0,1fr)]">
      <aside aria-label="Edit settings" className="flex min-h-0 flex-col border-r border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pt-4 pb-5">
          <div>
            <Segmented
              stretch
              ariaLabel="Edit mode"
              value={mode}
              onChange={(m) => dispatch({ type: "patchEdit", patch: { mode: m } })}
              options={[
                { value: "instruction" as EditMode, label: "Describe a change", title: "Tell it what to change; the rest stays" },
                { value: "restyle" as EditMode, label: "Restyle", title: "Redraw the whole image in a new look" },
              ]}
            />
            <p className="mt-1.5 text-xs text-neutral-500">
              {e.mode == null
                ? autoEdit
                  ? "Picked automatically — you have an edit model."
                  : "Picked automatically — Restyle works with your Create model."
                : mode === "instruction"
                  ? "Say what should change. Everything else stays the same."
                  : "Redraws the whole picture with your description."}
            </p>
          </div>

          {mode === "instruction" ? (
            needsEditModel ? (
              <div className="space-y-3">
                <div className="rounded-xl bg-neutral-50 p-3 text-sm dark:bg-neutral-800/50">
                  <div className="font-medium">{noGpu ? "Describing a change needs a graphics card" : "Get the best edit model for your GPU"}</div>
                  <p className="mt-0.5 text-xs text-neutral-500">
                    {noGpu
                      ? "Pinhole didn't find one it can use, and edit models are too big for the processor. Switch to Restyle — it works with the model you already have."
                      : "Edit models change just what you ask for. Or switch to Restyle — it works with the model you already have."}
                  </p>
                </div>
                <RecommendedCards roles={["edit"]} compact />
              </div>
            ) : (
              <>
                <ModelPicker models={edits} value={editModelId} onChange={(id) => dispatch({ type: "patchEdit", patch: { editModelId: id } })} label="Edit model" />
                <div>
                  <label htmlFor="edit-instruction" className="mb-1.5 block text-sm font-medium">
                    What should change?
                  </label>
                  <AutoTextarea
                    id="edit-instruction"
                    minRows={3}
                    maxRows={10}
                    value={e.instruction}
                    placeholder="e.g. make it evening with warm street lights, or replace the mug with a water bottle"
                    onChange={(ev) => dispatch({ type: "patchEdit", patch: { instruction: ev.target.value } })}
                  />
                </div>
                <div>
                  <div className="mb-1 text-sm text-neutral-600 dark:text-neutral-400">Stay close to original</div>
                  <Slider
                    ariaLabel="Stay close to original"
                    value={e.stayClose ?? defaultStickPosition(ui)}
                    onChange={(v) => dispatch({ type: "patchEdit", patch: { stayClose: v } })}
                    left="Loose"
                    right="Close"
                  />
                </div>
              </>
            )
          ) : (
            <>
              {creates.length ? (
                <ModelPicker models={creates} value={restyleModelId} onChange={(id) => dispatch({ type: "patchEdit", patch: { restyleModelId: id } })} label="Model" />
              ) : (
                <RecommendedCards roles={["realistic", "anime"]} compact />
              )}
              <div>
                <label htmlFor="edit-restyle" className="mb-1.5 block text-sm font-medium">
                  What should it look like?
                </label>
                <AutoTextarea
                  id="edit-restyle"
                  minRows={3}
                  maxRows={10}
                  value={e.restylePrompt}
                  placeholder="e.g. a watercolor painting of the same scene"
                  onChange={(ev) => dispatch({ type: "patchEdit", patch: { restylePrompt: ev.target.value } })}
                />
              </div>
              <div>
                <div className="mb-1.5 text-sm text-neutral-600 dark:text-neutral-400">How much to change</div>
                <Segmented
                  stretch
                  ariaLabel="How much to change"
                  value={e.change}
                  onChange={(v) => dispatch({ type: "patchEdit", patch: { change: v } })}
                  options={[
                    { value: "subtle" as ChangeAmount, label: "Subtle" },
                    { value: "medium" as ChangeAmount, label: "Medium" },
                    { value: "strong" as ChangeAmount, label: "Strong" },
                  ]}
                />
              </div>
            </>
          )}

          <div className="flex flex-wrap items-center gap-2">
            <StylePicker value={e.styleId} onChange={(id) => dispatch({ type: "patchEdit", patch: { styleId: id } })} familyId={model?.familyId} familyLabel={model?.familyLabel} />
            {e.styleId && mode === "instruction" && <span className="text-xs text-neutral-500">“make it look like: …”</span>}
          </div>

          <div className="rounded-xl border border-neutral-200 p-3 dark:border-neutral-800">
            <Toggle
              checked={maskOn}
              onChange={setMaskOn}
              label={
                <span className="inline-flex items-center gap-1.5 font-medium">
                  <Brush className="h-3.5 w-3.5" /> Only change here
                </span>
              }
              hint={maskOn ? "Paint over the part of the image that may change." : "Optional: paint the area to change."}
            />
            {maskOn && (
              <div className="mt-3 space-y-2.5">
                <div className="flex items-center gap-2">
                  <Segmented
                    size="sm"
                    ariaLabel="Brush or eraser"
                    value={erase ? "erase" : "paint"}
                    onChange={(v) => setErase(v === "erase")}
                    options={[
                      { value: "paint", label: (<><Brush className="h-3 w-3" /> Paint</>) },
                      { value: "erase", label: (<><Eraser className="h-3 w-3" /> Erase</>) },
                    ]}
                  />
                  <Button size="sm" variant="ghost" onClick={() => mask.current?.clear()} disabled={!painted}>
                    <Trash className="h-3.5 w-3.5" /> Clear
                  </Button>
                </div>
                <Slider ariaLabel="Brush size" min={6} max={160} step={1} value={brush} onChange={setBrush} left="Brush" right={<span className="tabular-nums">{brush}px</span>} />
              </div>
            )}
          </div>

          <section className="rounded-xl border border-neutral-200 dark:border-neutral-800">
            <button type="button" aria-expanded={moreOpen} onClick={() => setMoreOpen((o) => !o)} className={cx("flex w-full items-center gap-2 rounded-xl px-3 py-2.5 text-left text-sm font-medium", focusRing)}>
              <SlidersHorizontal className="h-4 w-4 text-neutral-500" /> Fine-tune
              <ChevronDown className={cx("ml-auto h-4 w-4 text-neutral-400 transition-transform", moreOpen && "rotate-180")} />
            </button>
            {moreOpen && (
              <div className="space-y-3 border-t border-neutral-200 px-3 pt-3 pb-4 dark:border-neutral-800">
                <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-2 text-sm">
                  <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">Quality</span>
                  <Segmented size="sm" ariaLabel="Quality" value={e.quality} onChange={(q) => dispatch({ type: "patchEdit", patch: { quality: q } })} options={(["fast", "balanced", "best"] as Quality[]).map((q, i) => ({ value: q, label: q[0].toUpperCase() + q.slice(1), title: ui ? `${ui.qualitySteps[i]} steps` : undefined }))} />
                  <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">Output size</span>
                  <div className="flex items-center gap-2">
                    <Segmented size="sm" ariaLabel="Output size" value={size} onChange={setSize} options={[{ value: "smaller" as SizeChoice, label: "Smaller" }, { value: "normal" as SizeChoice, label: "Normal" }, { value: "larger" as SizeChoice, label: "Larger" }]} />
                    {outSize && <span className="text-xs text-neutral-500 tabular-nums">{outSize[0]}×{outSize[1]}</span>}
                  </div>
                  <label htmlFor="edit-seed" className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
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
                      dispatch({ type: "patchEdit", patch: { seed: Number.isFinite(n) && n >= 0 ? n : null } });
                    }}
                  />
                </div>
                <p className="text-[11px] text-neutral-400">Size keeps your image’s shape. {ui ? `${ui.label} defaults are used for everything else.` : ""}</p>
              </div>
            )}
          </section>
        </div>

        <div className="shrink-0 space-y-2 border-t border-neutral-200 px-5 py-4 dark:border-neutral-800">
          {myJob ? (
            <JobProgress
              job={myJob}
              cancelling={cancelling}
              onCancel={async () => {
                setCancelling(true);
                await actions.cancel();
                setCancelling(false);
              }}
            />
          ) : (
            <Button variant="primary" size="lg" className="w-full" disabled={!canRun} onClick={() => void run()}>
              <WandSparkles className="h-4 w-4" />
              {mode === "instruction" ? "Apply edit" : "Restyle"}
              <span className="ml-1 inline-flex gap-0.5 opacity-70">
                <Kbd>{modKey}</Kbd>
                <Kbd>Enter</Kbd>
              </span>
            </Button>
          )}
          {!current && !myJob && <p className="text-center text-xs text-neutral-500">Add an image to start.</p>}
          {job && !myJob && <p className="text-center text-xs text-neutral-500">Busy creating — editing is available when it finishes.</p>}
          {error && <ErrorWithFix error={error} onDismiss={() => setError(null)} onRetry={() => void run()} />}
        </div>
      </aside>

      <DropTarget onFile={(f) => void load(f)} className="flex min-h-0 min-w-0 flex-col bg-neutral-100 dark:bg-neutral-950" label="Drop to edit this image">
        {picker.input}
        {!current ? (
          <div className="flex min-h-0 flex-1 p-6">
            <DropZone onFile={(f) => void load(f)} title="Add an image to edit" busy={importing}>
              {importing && <Spinner className="mt-3 h-4 w-4" />}
            </DropZone>
          </div>
        ) : (
          <>
            <div className="flex shrink-0 items-center gap-1.5 border-b border-neutral-200 bg-white/60 px-4 py-2 dark:border-neutral-800 dark:bg-neutral-900/40">
              <IconButton label="Undo" disabled={e.index === 0} onClick={() => dispatch({ type: "editGoto", index: e.index - 1 })}>
                <Undo2 className="h-4 w-4" />
              </IconButton>
              <IconButton label="Redo" disabled={e.index >= e.chain.length - 1} onClick={() => dispatch({ type: "editGoto", index: e.index + 1 })}>
                <Redo2 className="h-4 w-4" />
              </IconButton>
              <IconButton label="Delete this edit" disabled={e.index === 0 || !!job} onClick={() => dispatch({ type: "editDelete", index: e.index })}>
                <Trash2 className="h-4 w-4" />
              </IconButton>
              <span className="mx-1 h-5 w-px bg-neutral-200 dark:bg-neutral-800" />
              <Button size="sm" variant={compare && before ? "secondary" : "ghost"} disabled={!before && e.index === 0} aria-pressed={compare && !!before} onClick={() => setCompare((c) => !c)}>
                <Columns2 className="h-3.5 w-3.5" /> Compare
              </Button>
              {compare && e.index > 1 && (
                <Segmented size="sm" ariaLabel="Compare with" value={compareWith} onChange={setCompareWith} options={[{ value: "previous", label: "Previous" }, { value: "original", label: "Original" }]} />
              )}
              <div className="ml-auto flex items-center gap-1.5">
                <Button size="sm" variant="ghost" onClick={picker.open} title="Edit a different image" aria-label="New image">
                  <ImagePlus className="h-3.5 w-3.5" /> <span className="hidden xl:inline">New image</span>
                </Button>
                <Button size="sm" onClick={() => void actions.save(current.id).catch((err) => setError(api.asCoreError(err)))}>
                  <Save className="h-3.5 w-3.5" /> Save
                </Button>
                <IconButton label="Copy image" size="sm" variant="secondary" onClick={() => void actions.copyImage(current.id).catch((err) => setError(api.asCoreError(err)))}>
                  <Copy className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton label="Describe this image" size="sm" variant="secondary" onClick={() => actions.sendToDescribe(current.id)}>
                  <ScanText className="h-3.5 w-3.5" />
                </IconButton>
              </div>
            </div>

            <Stage
              current={current}
              before={compare ? before : undefined}
              beforeLabel={compareWith === "original" ? "Original" : (prevNode?.label ?? "Before")}
              afterLabel={node?.label ?? "After"}
              maskOn={maskOn && !(compare && before)}
              maskRef={mask}
              brush={brush}
              erase={erase}
              onPainted={setPainted}
            />
            {node?.meta && <p className="-mt-3 shrink-0 pb-3 text-center text-xs text-neutral-500 tabular-nums">{settingsSummary(node.meta)}</p>}

            <div className="shrink-0 border-t border-neutral-200 bg-white/60 px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900/40">
              <ol className="flex items-center gap-1.5 overflow-x-auto pb-1" aria-label="Edit history">
                {e.chain.map((n, i) => {
                  const img = images[n.imageId];
                  return (
                    <li key={n.imageId} className="flex shrink-0 items-center gap-1.5">
                      {i > 0 && <ArrowRight className="h-3.5 w-3.5 text-neutral-400" aria-hidden />}
                      <button
                        type="button"
                        aria-current={i === e.index ? "step" : undefined}
                        onClick={() => dispatch({ type: "editGoto", index: i })}
                        className={cx("group flex flex-col items-center gap-1 rounded-lg p-1", focusRing, i === e.index ? "bg-amber-50 dark:bg-amber-500/10" : "hover:bg-neutral-100 dark:hover:bg-neutral-800")}
                      >
                        {img && <img src={img.url} alt="" className={cx("h-14 w-14 rounded-md object-cover ring-2", i === e.index ? "ring-amber-500" : "ring-transparent")} draggable={false} />}
                        <span className={cx("text-[11px]", i === e.index ? "font-medium text-amber-900 dark:text-amber-200" : "text-neutral-500")}>{n.label}</span>
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
}) {
  const container = useRef<HTMLDivElement>(null);
  const box = useFitBox(container, current.width, current.height);
  return (
    <div ref={container} className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden p-6">
      {box.width > 0 && before && (
        <CompareView before={before} after={current} width={box.width} height={box.height} beforeLabel={beforeLabel} afterLabel={afterLabel} />
      )}
      {/* Kept mounted while comparing so a painted mask isn't lost. */}
      <div
        hidden={!!before || box.width === 0}
        className="relative overflow-hidden rounded-lg shadow-lg ring-1 ring-black/5 dark:ring-white/10"
        style={{ width: box.width, height: box.height }}
      >
        <img src={current.url} alt={`Image being edited (${afterLabel})`} className="absolute inset-0 h-full w-full" draggable={false} />
        <MaskCanvas ref={maskRef} width={current.width} height={current.height} displayWidth={box.width} brush={brush} erase={erase} active={maskOn} onPaintedChange={onPainted} />
      </div>
    </div>
  );
}
