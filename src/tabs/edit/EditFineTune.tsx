// Edit's Fine-tune section: quality, output size, seed, upscaler, LoRAs and the prompt preview.
import type { Dispatch, SetStateAction } from "react";
import { ChevronDown, SlidersHorizontal } from "lucide-react";
import { UpscalerSelect, useUpscaler } from "../../components/UpscalerChoice";
import {
  Badge,
  Segmented,
  cx,
  focusRing,
  inputClass,
} from "../../components/ui";
import type {
  FamilyUi,
  GenerateRequest,
  InstalledModel,
  Quality,
} from "../../lib/types";
import type { Action, EditMode, EditParams } from "../../lib/state/model";
import type { EditSizeChoice } from "../../lib/state/request";
import { LoraSection, PromptPreview } from "../create/FineTune";

type SizeChoice = EditSizeChoice;

export function EditFineTune({
  e,
  mode,
  fixing,
  extending,
  painted,
  model,
  ui,
  moreOpen,
  setMoreOpen,
  fineTuneChanged,
  size,
  setSize,
  outSize,
  upscaler,
  previewReq,
  dispatch,
}: {
  e: EditParams;
  mode: EditMode;
  fixing: boolean;
  extending: boolean;
  painted: boolean;
  model: InstalledModel | null;
  ui: FamilyUi | null;
  moreOpen: boolean;
  setMoreOpen: Dispatch<SetStateAction<boolean>>;
  fineTuneChanged: number;
  size: SizeChoice;
  setSize: (s: SizeChoice) => void;
  outSize: [number, number] | null;
  upscaler: ReturnType<typeof useUpscaler>;
  previewReq: GenerateRequest | null;
  dispatch: (a: Action) => void;
}) {
  return (
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
        {fineTuneChanged > 0 && (
          <Badge tone="amber">{fineTuneChanged} changed</Badge>
        )}
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
            <span className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
              Upscaler
            </span>
            <UpscalerSelect upscaler={upscaler} />
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
              ? painted
                ? "The picture keeps its size; only the painted spot changes. "
                : "The picture keeps its size; each face is redrawn larger, then blended back in. "
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
  );
}
