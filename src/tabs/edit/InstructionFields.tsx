// Describe a change: the edit model (or edit models to get) and the instruction.
import { AutoTextarea, Slider } from "../../components/ui";
import { ModelPicker } from "../../components/ModelPicker";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { defaultStayClosePosition } from "../../lib/paste/map";
import type { FamilyUi, InstalledModel } from "../../lib/types";
import type { Action, EditParams } from "../../lib/state/model";

// Recommended edit models that can take a second image (registry `multi_ref`; FLUX.2 has no one-click download yet).
const TWO_IMAGE_PICKS = ["qwen_image_21"];

export function InstructionFields({
  e,
  ui,
  noGpu,
  twoImages,
  needsEditModel,
  edits,
  editModelId,
  editFit,
  dispatch,
}: {
  e: EditParams;
  ui: FamilyUi | null;
  noGpu: boolean;
  twoImages: boolean;
  needsEditModel: boolean;
  edits: InstalledModel[];
  editModelId: string | null;
  editFit: InstalledModel["fit"] | null;
  dispatch: (a: Action) => void;
}) {
  return needsEditModel ? (
    <div className="space-y-3">
      <div className="rounded-xl bg-neutral-50 p-3 text-sm dark:bg-neutral-800/50">
        <div className="font-medium">
          {noGpu
            ? "Describing a change needs a graphics card"
            : twoImages
              ? "Combining two images needs another edit model"
              : "Get the best edit model for your graphics card"}
        </div>
        <p className="mt-0.5 text-xs text-neutral-500">
          {noGpu
            ? "Pinhole didn't find one it can use, and edit models are too big for the processor. Switch to Restyle — it works with the model you already have."
            : twoImages
              ? "Qwen-Image 2.1, Qwen Image Edit and FLUX.2 models can use a second image. Or remove the second image to edit with the model you have."
              : "Edit models change just what you ask for. Or switch to Restyle — it works with the model you already have."}
        </p>
      </div>
      <RecommendedCards
        roles={["edit"]}
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
          roles={["edit"]}
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
  );
}
