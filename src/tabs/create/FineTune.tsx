// Fine-tune drawer (SPEC §5.1): every automatic choice, visible and overridable.
// Each field shows the registry default and a reset button.
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { ChevronDown, Dices, Plus, RotateCcw, SlidersHorizontal, TriangleAlert, X } from "lucide-react";
import { AutoTextarea, Badge, IconButton, MenuItem, MenuLabel, Popover, Segmented, Select, Toggle, cx, focusRing, inputClass } from "../../components/ui";
import { namedSizes, screenPixels, sizeForRatio } from "../../lib/sizes";
import { SD_SAMPLERS, SD_SCHEDULERS, defaultStickPosition, samplerLabel, schedulerLabel, stickValue } from "../../lib/paste/map";
import { asCoreError, previewFinalPrompt } from "../../lib/api";
import type { FamilyUi, FineTune, GenerateRequest, InstalledModel } from "../../lib/types";
import { useActions } from "../../lib/state/AppProvider";
import { useDebounced } from "../../lib/state/hooks";
import { DEFAULT_LORA_WEIGHT, loraCompatible } from "../../lib/state/model";
import { requestAddonBrowse } from "../models/lib/session";
import { buildCreateRequest } from "../../lib/state/request";
import { useAppState, useDispatch } from "../../lib/state/store";
import { FALLBACK_SHAPES, qualityIndex } from "./Dials";
import { UpscalerSegmented, useUpscaler } from "../../components/UpscalerChoice";

function Row({
  label,
  def,
  changed,
  onReset,
  children,
  full,
  htmlFor,
}: {
  label: ReactNode;
  def?: ReactNode;
  changed: boolean;
  onReset: () => void;
  children: ReactNode;
  full?: boolean;
  htmlFor?: string;
}) {
  return (
    <div className={cx("min-w-0", full && "col-span-2")}>
      <div className="mb-1 flex h-5 items-center justify-between gap-2">
        <label htmlFor={htmlFor} className={cx("truncate text-xs font-medium", changed ? "text-amber-800 dark:text-amber-300" : "text-neutral-600 dark:text-neutral-400")}>
          {label}
        </label>
        <span className="flex min-w-0 items-center gap-1">
          {def != null && <span className="truncate text-[11px] text-neutral-400">default {def}</span>}
          {changed && (
            <button type="button" onClick={onReset} title="Reset to default" aria-label={`Reset ${typeof label === "string" ? label : "field"}`} className={cx("rounded p-0.5 text-neutral-400 hover:text-neutral-800 dark:hover:text-white", focusRing)}>
              <RotateCcw className="h-3 w-3" />
            </button>
          )}
        </span>
      </div>
      {children}
    </div>
  );
}

function NumberInput({
  id,
  value,
  placeholder,
  onChange,
  step = 1,
  min,
  max,
  integer = false,
}: {
  id: string;
  value: number | null | undefined;
  placeholder: string;
  onChange: (v: number | null) => void;
  step?: number;
  min?: number;
  max?: number;
  integer?: boolean;
}) {
  const [text, setText] = useState(value != null ? String(value) : "");
  useEffect(() => {
    setText((t) => (value == null ? "" : Number(t) === value ? t : String(value)));
  }, [value]);
  return (
    <input
      id={id}
      type="number"
      inputMode={integer ? "numeric" : "decimal"}
      className={cx(inputClass, "h-9 py-0 tabular-nums")}
      value={text}
      step={step}
      min={min}
      max={max}
      placeholder={placeholder}
      onChange={(e) => {
        setText(e.target.value);
        const raw = e.target.value.trim();
        if (!raw) return onChange(null);
        const n = integer ? Number.parseInt(raw, 10) : Number.parseFloat(raw);
        if (Number.isFinite(n)) onChange(n);
      }}
    />
  );
}

type Tri = "auto" | "on" | "off";
const tri = (v: boolean | null | undefined): Tri => (v == null ? "auto" : v ? "on" : "off");
const fromTri = (t: Tri): boolean | null => (t === "auto" ? null : t === "on");

/** "My screen", "Phone", "Instagram", "Thumbnail": one click sets Width and Height. */
function NamedSizeChips({ ui, width, height, onPick }: { ui: FamilyUi | null; width: number | null; height: number | null; onPick: (w: number, h: number) => void }) {
  // Read on every render: the window may have moved to another monitor.
  const screen = screenPixels();
  const sizes = namedSizes(screen);
  // Two names can give the same size (a 16:9 screen and Thumbnail): keep the one just clicked lit.
  const [picked, setPicked] = useState<string | null>(null);
  const matching = sizes.filter((n) => {
    const [w, h] = sizeForRatio(n.ratio, ui);
    return w === width && h === height;
  });
  const active = matching.find((n) => n.id === picked) ?? matching[0];
  return (
    <div>
      <div role="group" aria-label="Named sizes" className="flex flex-wrap gap-1.5">
        {sizes.map((n) => {
          const [w, h] = sizeForRatio(n.ratio, ui);
          const on = active?.id === n.id;
          return (
            <button
              key={n.id}
              type="button"
              aria-pressed={on}
              title={`${n.note}: ${w}×${h}`}
              onClick={() => {
                setPicked(n.id);
                onPick(w, h);
              }}
              className={cx(
                "h-8 rounded-lg border px-2.5 text-xs transition-colors",
                focusRing,
                on
                  ? "border-amber-500 bg-amber-50 font-medium text-amber-950 dark:border-amber-500/70 dark:bg-amber-500/10 dark:text-amber-100"
                  : "border-neutral-200 bg-white text-neutral-600 hover:border-neutral-300 hover:text-neutral-900 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400 dark:hover:border-neutral-600 dark:hover:text-white",
              )}
            >
              {n.label}
            </button>
          );
        })}
      </div>
      {active?.id === "screen" && screen && (
        <p className="mt-1 text-[11px] text-neutral-500">
          Made at {width}×{height}, your screen’s shape. Use Upscale on the picture for a bigger one.
        </p>
      )}
    </div>
  );
}

/** Which upscaler Upscale uses (a saved setting, shared with Edit's Fine-tune). */
function UpscalerRow() {
  const upscaler = useUpscaler();
  return (
    <Row full label="Upscaler (for Upscale)" def="auto" changed={upscaler.value !== "auto"} onReset={() => upscaler.choose("auto")}>
      <UpscalerSegmented upscaler={upscaler} />
    </Row>
  );
}

export function FineTuneDrawer({ ui, model }: { ui: FamilyUi | null; model: InstalledModel | null }) {
  const [open, setOpen] = useState(false);
  const c = useAppState((s) => s.create);
  const styles = useAppState((s) => s.styles);
  const dispatch = useDispatch();
  const ft = c.fineTune;
  const set = (patch: Partial<FineTune>) => dispatch({ type: "setFineTune", patch });

  const changedKeys = Object.keys(ft).filter((k) => k !== "negativePrompt" || !!ft.negativePrompt?.trim());
  const changedCount = changedKeys.length + (c.loras.length ? 1 : 0);

  const stickPos = c.stick ?? defaultStickPosition(ui);
  const steps = ui ? ui.qualitySteps[qualityIndex(c.quality)] : null;
  const [sw, sh] = ui?.shapes[c.shape] ?? FALLBACK_SHAPES[c.shape];
  const cfgDefault = ui ? (ui.stickMapsTo === "cfg" && ui.showStick ? stickValue(ui, stickPos) : ui.defaultCfg) : null;
  const guidanceDefault = ui ? (ui.stickMapsTo === "guidance" ? stickValue(ui, stickPos) : ui.defaultGuidance) : null;
  const showGuidance = !ui || ui.stickMapsTo === "guidance" || ui.defaultGuidance != null;
  const showNegative = !ui || ui.usesNegativePrompt;
  const hiresDefaultOn = !!ui?.hiresAtBest && c.quality === "best";
  const hiresOn = ft.hires ?? hiresDefaultOn;
  const style = styles.find((s) => s.id === c.styleId) ?? null;

  return (
    <section className="rounded-xl border border-neutral-200 dark:border-neutral-800">
      <div className="flex items-center gap-2 pr-2">
        <button
          type="button"
          aria-expanded={open}
          aria-controls="finetune-panel"
          onClick={() => setOpen((o) => !o)}
          className={cx("flex min-w-0 flex-1 items-center gap-2 rounded-xl px-3 py-2.5 text-left text-sm font-medium", focusRing)}
        >
          <SlidersHorizontal className="h-4 w-4 text-neutral-500" />
          Fine-tune
          {changedCount > 0 && <Badge tone="amber">{changedCount} changed</Badge>}
          <ChevronDown className={cx("ml-auto h-4 w-4 text-neutral-400 transition-transform", open && "rotate-180")} />
        </button>
        {open && changedCount > 0 && (
          <button
            type="button"
            className={cx("shrink-0 rounded px-1.5 py-1 text-xs text-neutral-500 hover:text-neutral-900 dark:hover:text-white", focusRing)}
            onClick={() => {
              dispatch({ type: "patchCreate", patch: { fineTune: ft.negativePrompt ? { negativePrompt: ft.negativePrompt } : {}, loras: [] } });
            }}
            title="Reset everything except your negative prompt"
          >
            Reset all
          </button>
        )}
      </div>

      {open && (
        <div id="finetune-panel" className="grid grid-cols-2 gap-x-3 gap-y-3.5 border-t border-neutral-200 px-3 pt-3 pb-4 dark:border-neutral-800">
          {showNegative && (
            <Row
              full
              htmlFor="ft-negative"
              label="Negative prompt (things to avoid)"
              changed={!!ft.negativePrompt}
              onReset={() => set({ negativePrompt: null })}
            >
              <AutoTextarea
                id="ft-negative"
                minRows={2}
                maxRows={6}
                value={ft.negativePrompt ?? ""}
                placeholder={ui?.defaultNegativePrompt ? `Default: ${ui.defaultNegativePrompt}` : "Nothing"}
                onChange={(e) => set({ negativePrompt: e.target.value || null })}
              />
              {style?.negative && <p className="mt-1 text-[11px] text-neutral-500">The “{style.name}” style also avoids: {style.negative}</p>}
            </Row>
          )}

          <Row label="Sampler" htmlFor="ft-sampler" def={ui ? samplerLabel(ui.defaultSampler) : undefined} changed={ft.sampler != null} onReset={() => set({ sampler: null })}>
            <Select ariaLabel="Sampler" value={ft.sampler ?? ""} onChange={(v) => set({ sampler: v || null })}>
              <option value="">Default{ui?.defaultSampler ? ` (${samplerLabel(ui.defaultSampler)})` : ""}</option>
              {SD_SAMPLERS.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </Select>
          </Row>
          <Row label="Scheduler" htmlFor="ft-scheduler" def={ui ? schedulerLabel(ui.defaultScheduler) : undefined} changed={ft.scheduler != null} onReset={() => set({ scheduler: null })}>
            <Select ariaLabel="Scheduler" value={ft.scheduler ?? ""} onChange={(v) => set({ scheduler: v || null })}>
              <option value="">Default{ui?.defaultScheduler ? ` (${schedulerLabel(ui.defaultScheduler)})` : ""}</option>
              {SD_SCHEDULERS.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </Select>
          </Row>

          <Row label="Steps" htmlFor="ft-steps" def={steps ?? undefined} changed={ft.steps != null} onReset={() => set({ steps: null })}>
            <NumberInput id="ft-steps" integer min={1} max={150} value={ft.steps} placeholder={steps != null ? String(steps) : "Auto"} onChange={(v) => set({ steps: v })} />
          </Row>
          <Row label="CFG" htmlFor="ft-cfg" def={cfgDefault ?? undefined} changed={ft.cfg != null} onReset={() => set({ cfg: null })}>
            <NumberInput id="ft-cfg" step={0.5} min={1} max={30} value={ft.cfg} placeholder={cfgDefault != null ? String(cfgDefault) : "Auto"} onChange={(v) => set({ cfg: v })} />
          </Row>

          {showGuidance && (
            <Row label="Guidance" htmlFor="ft-guidance" def={guidanceDefault ?? undefined} changed={ft.guidance != null} onReset={() => set({ guidance: null })}>
              <NumberInput id="ft-guidance" step={0.1} min={0} max={30} value={ft.guidance} placeholder={guidanceDefault != null ? String(guidanceDefault) : "Auto"} onChange={(v) => set({ guidance: v })} />
            </Row>
          )}
          <Row label="Seed" htmlFor="ft-seed" def="random" changed={ft.seed != null} onReset={() => set({ seed: null })}>
            <div className="flex gap-1.5">
              <NumberInput id="ft-seed" integer min={0} value={ft.seed} placeholder="Random" onChange={(v) => set({ seed: v })} />
              <IconButton label="Pick a random seed" variant="secondary" onClick={() => set({ seed: Math.floor(Math.random() * 2 ** 31) })}>
                <Dices className="h-4 w-4" />
              </IconButton>
            </div>
          </Row>

          <Row full label="Named sizes" def={`${sw}×${sh}`} changed={ft.width != null || ft.height != null} onReset={() => set({ width: null, height: null })}>
            <NamedSizeChips ui={ui} width={ft.width ?? null} height={ft.height ?? null} onPick={(w, h) => set({ width: w, height: h })} />
          </Row>
          <Row label="Width" htmlFor="ft-width" def={sw} changed={ft.width != null} onReset={() => set({ width: null })}>
            <NumberInput id="ft-width" integer step={64} min={256} max={4096} value={ft.width} placeholder={String(sw)} onChange={(v) => set({ width: v })} />
          </Row>
          <Row label="Height" htmlFor="ft-height" def={sh} changed={ft.height != null} onReset={() => set({ height: null })}>
            <NumberInput id="ft-height" integer step={64} min={256} max={4096} value={ft.height} placeholder={String(sh)} onChange={(v) => set({ height: v })} />
          </Row>

          <Row label="Flow shift" htmlFor="ft-flow" def={ui?.defaultFlowShift ?? "auto"} changed={ft.flowShift != null} onReset={() => set({ flowShift: null })}>
            <NumberInput id="ft-flow" step={0.1} min={0} max={20} value={ft.flowShift} placeholder={ui?.defaultFlowShift != null ? String(ui.defaultFlowShift) : "Auto"} onChange={(v) => set({ flowShift: v })} />
          </Row>
          {(!ui || ui.defaultClipSkip != null) && (
            <Row label="Clip skip" htmlFor="ft-clip" def={ui?.defaultClipSkip ?? undefined} changed={ft.clipSkip != null} onReset={() => set({ clipSkip: null })}>
              <NumberInput id="ft-clip" integer min={1} max={12} value={ft.clipSkip} placeholder={ui?.defaultClipSkip != null ? String(ui.defaultClipSkip) : "Auto"} onChange={(v) => set({ clipSkip: v })} />
            </Row>
          )}

          <Row full label="Hires fix" def={hiresDefaultOn ? "on at Best" : "off"} changed={ft.hires != null} onReset={() => set({ hires: null, hiresScale: null, hiresDenoise: null })}>
            <div className="flex flex-wrap items-center gap-3">
              <Segmented size="sm" ariaLabel="Hires fix" value={tri(ft.hires)} onChange={(t) => set({ hires: fromTri(t) })} options={[{ value: "auto" as Tri, label: "Auto" }, { value: "on" as Tri, label: "On" }, { value: "off" as Tri, label: "Off" }]} />
              {hiresOn && (
                <div className="flex items-center gap-2 text-xs text-neutral-500">
                  <label htmlFor="ft-hscale">Scale</label>
                  <div className="w-20">
                    <NumberInput id="ft-hscale" step={0.25} min={1} max={4} value={ft.hiresScale} placeholder="1.5" onChange={(v) => set({ hiresScale: v })} />
                  </div>
                  <label htmlFor="ft-hden">Strength</label>
                  <div className="w-20">
                    <NumberInput id="ft-hden" step={0.05} min={0} max={1} value={ft.hiresDenoise} placeholder="0.45" onChange={(v) => set({ hiresDenoise: v })} />
                  </div>
                </div>
              )}
            </div>
          </Row>

          <Row full label="VAE tiling (saves VRAM, a bit slower)" def="auto" changed={ft.vaeTiling != null} onReset={() => set({ vaeTiling: null })}>
            <Segmented size="sm" ariaLabel="VAE tiling" value={tri(ft.vaeTiling)} onChange={(t) => set({ vaeTiling: fromTri(t) })} options={[{ value: "auto" as Tri, label: "Auto" }, { value: "on" as Tri, label: "On" }, { value: "off" as Tri, label: "Off" }]} />
          </Row>

          <UpscalerRow />

          {ui?.autoPromptPrefix && (
            <Row full label="Automatic prompt prefix" def="on" changed={ft.autoPromptPrefix != null} onReset={() => set({ autoPromptPrefix: null })}>
              <Toggle
                checked={ft.autoPromptPrefix ?? true}
                onChange={(v) => set({ autoPromptPrefix: v })}
                label={<span className="text-sm">Add “{ui.autoPromptPrefix.trim().replace(/,$/, "")}” in front</span>}
              />
            </Row>
          )}

          <LoraSection model={model} />

          <FinalPromptPreview ui={ui} model={model} />
          <p className="col-span-2 text-[11px] text-neutral-400">Pinhole picks all of these from the model’s registry entry. Moving a simple dial above hands control back to it.</p>
          {!model && <p className="col-span-2 text-xs text-neutral-500">Pick a model to see its defaults.</p>}
        </div>
      )}
    </section>
  );
}

/** Add-on list with Add; `target`: the Create or the Edit tab's add-ons. */
export function LoraSection({ model, target = "create" }: { model: InstalledModel | null; target?: "create" | "edit" }) {
  const c = useAppState((s) => s[target]);
  const loras = useAppState((s) => s.loras);
  const dispatch = useDispatch();
  const actions = useActions();
  const setLoras = (list: typeof c.loras) => dispatch(target === "edit" ? { type: "patchEdit", patch: { loras: list } } : { type: "patchCreate", patch: { loras: list } });
  const available = loras.filter((l) => !c.loras.some((u) => u.loraId === l.id));
  const compatible = available.filter((l) => loraCompatible(l, model?.familyId));
  const incompatible = available.filter((l) => !loraCompatible(l, model?.familyId));

  return (
    <div className="col-span-2">
      <div className="mb-1.5 flex h-5 items-center justify-between">
        <span className={cx("text-xs font-medium", c.loras.length ? "text-amber-800 dark:text-amber-300" : "text-neutral-600 dark:text-neutral-400")}>LoRAs (style add-ons)</span>
        <Popover
          align="end"
          width={320}
          trigger={(p) => (
            <button {...p} type="button" className={cx("inline-flex items-center gap-1 rounded px-1 text-xs font-medium text-amber-700 hover:underline dark:text-amber-400", focusRing)}>
              <Plus className="h-3.5 w-3.5" /> Add
            </button>
          )}
        >
          {(close) => (
            <div>
              {compatible.map((l) => (
                <MenuItem
                  key={l.id}
                  onClick={() => {
                    setLoras([...c.loras, { loraId: l.id, weight: DEFAULT_LORA_WEIGHT }]);
                    close();
                  }}
                  hint={l.trainedWords.length ? `Trigger: ${l.trainedWords.join(", ")}` : l.baseModel ?? undefined}
                >
                  {l.friendlyName}
                </MenuItem>
              ))}
              {!compatible.length && <div className="px-2.5 py-2 text-sm text-neutral-500">No add-ons for this model yet.</div>}
              {incompatible.length > 0 && <MenuLabel>For other models</MenuLabel>}
              {incompatible.map((l) => (
                <MenuItem key={l.id} disabled hint={l.baseModel ?? undefined}>
                  {l.friendlyName}
                </MenuItem>
              ))}
              <div className="mt-1 border-t border-neutral-200 pt-1 dark:border-neutral-800">
                <MenuItem
                  onClick={() => {
                    close();
                    if (model) requestAddonBrowse(model.id);
                    actions.setTab("models");
                  }}
                >
                  {model ? `Find add-ons for ${model.friendlyName}…` : "Find style add-ons…"}
                </MenuItem>
              </div>
            </div>
          )}
        </Popover>
      </div>
      {c.loras.length === 0 ? (
        <p className="text-xs text-neutral-400">None</p>
      ) : (
        <ul className="space-y-2">
          {c.loras.map((u, i) => {
            const l = loras.find((x) => x.id === u.loraId);
            const ok = !!l && loraCompatible(l, model?.familyId);
            return (
              <li key={u.loraId} className="rounded-lg border border-neutral-200 px-2.5 py-2 dark:border-neutral-800">
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm">{l?.friendlyName ?? "Missing add-on"}</span>
                  <input
                    type="number"
                    aria-label={`Weight for ${l?.friendlyName ?? "add-on"}`}
                    className={cx(inputClass, "h-7 w-18 px-2 py-0 text-xs tabular-nums")}
                    step={0.05}
                    min={-2}
                    max={3}
                    value={u.weight}
                    onChange={(e) => {
                      const w = Number.parseFloat(e.target.value);
                      if (Number.isFinite(w)) setLoras(c.loras.map((x, j) => (j === i ? { ...x, weight: w } : x)));
                    }}
                  />
                  <IconButton label={`Remove ${l?.friendlyName ?? "add-on"}`} size="sm" onClick={() => setLoras(c.loras.filter((_, j) => j !== i))}>
                    <X className="h-3.5 w-3.5" />
                  </IconButton>
                </div>
                <input
                  type="range"
                  aria-label={`Strength of ${l?.friendlyName ?? "add-on"}`}
                  className="pinhole-range mt-1 w-full"
                  min={0}
                  max={1.5}
                  step={0.05}
                  value={Math.min(1.5, Math.max(0, u.weight))}
                  onChange={(e) => setLoras(c.loras.map((x, j) => (j === i ? { ...x, weight: Number(e.target.value) } : x)))}
                />
                {!ok && (
                  <p className="mt-1 flex items-center gap-1 text-[11px] text-amber-700 dark:text-amber-400">
                    <TriangleAlert className="h-3 w-3" />
                    {l ? `Made for ${l.baseModel ?? "other"} models — it won’t be used with this one.` : "This add-on isn’t installed any more."}
                  </p>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

/** Read-only "Final prompt sent to the model" (combined in Rust, in memory). */
export function FinalPromptPreview({ ui, model }: { ui: FamilyUi | null; model: InstalledModel | null }) {
  const c = useAppState((s) => s.create);
  const loras = useAppState((s) => s.loras);
  const settings = useAppState((s) => s.settings);
  const req = useMemo(() => (model ? buildCreateRequest(c, { ui, loras, model, settings }) : null), [c, ui, loras, model, settings]);
  return <PromptPreview req={c.prompt.trim() ? req : null} empty="Type a prompt to see exactly what is sent." />;
}

/**
 * Shows what Rust sends for `req` (style, trigger words and prefix combined). `req` null = nothing to
 * send yet; `empty` says what to type. Shared by Create and Edit.
 */
export function PromptPreview({ req, empty }: { req: GenerateRequest | null; empty: string }) {
  // Rust resolves the style's words from its id, so an edit to the selected style must refresh too.
  const style = useAppState((s) => (req?.styleId ? s.styles.find((x) => x.id === req.styleId) : undefined));
  // Only the parts that change the text matter; debounce typing. (In memory only.)
  const key = req
    ? JSON.stringify([
        req.modelId,
        req.mode,
        req.prompt,
        req.styleId,
        style?.positive ?? null,
        style?.negative ?? null,
        req.fineTune.negativePrompt ?? null,
        req.fineTune.autoPromptPrefix ?? null,
        req.loras,
        req.addTriggerWords,
      ])
    : "";
  const debouncedKey = useDebounced(key, 350);
  const [preview, setPreview] = useState<{ prompt: string; negative: string | null } | null>(null);
  // Why the preview failed: the word check's own message, else a generic line.
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    if (!req) {
      setPreview(null);
      return;
    }
    let alive = true;
    previewFinalPrompt(req)
      .then((p) => {
        if (alive) {
          setPreview(p);
          setFailed(null);
        }
      })
      .catch((e) => {
        if (!alive) return;
        const err = asCoreError(e);
        setFailed(err.code === "blocked" ? err.message : "Preview isn’t available right now.");
      });
    return () => {
      alive = false;
    };
  }, [debouncedKey]);

  return (
    <div className="col-span-2">
      <div className="mb-1 text-xs font-medium text-neutral-600 dark:text-neutral-400">Final prompt sent to the model</div>
      <div className="max-h-40 overflow-auto rounded-lg bg-neutral-100 px-3 py-2 font-mono text-[11.5px] leading-relaxed text-neutral-700 select-text dark:bg-neutral-800/70 dark:text-neutral-300">
        {!req ? (
          <span className="text-neutral-400">{empty}</span>
        ) : failed ? (
          <span className="text-neutral-400">{failed}</span>
        ) : !preview ? (
          <span className="text-neutral-400">…</span>
        ) : (
          <>
            <div className="break-words whitespace-pre-wrap">{preview.prompt}</div>
            {preview.negative && (
              <div className="mt-2 break-words whitespace-pre-wrap text-neutral-500">
                <span className="font-sans font-medium">Avoid:</span> {preview.negative}
              </div>
            )}
          </>
        )}
      </div>
      <p className="mt-1 text-[11px] text-neutral-400">What the model receives for each image.</p>
    </div>
  );
}
