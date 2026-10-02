// Restyle, Fix details and Extend: the model, the description and how much to change.
import { AutoTextarea, Segmented } from "../../components/ui";
import { ModelPicker } from "../../components/ModelPicker";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import type { FamilyUi, InstalledModel } from "../../lib/types";
import type {
  Action,
  ChangeAmount,
  EditParams,
  ImgRef,
} from "../../lib/state/model";
import { ExtendControls } from "./ExtendControls";

export function RedrawFields({
  e,
  creates,
  restyleModelId,
  fixing,
  extending,
  current,
  ui,
  dispatch,
}: {
  e: EditParams;
  creates: InstalledModel[];
  restyleModelId: string | null;
  fixing: boolean;
  extending: boolean;
  current: ImgRef | undefined;
  ui: FamilyUi | null;
  dispatch: (a: Action) => void;
}) {
  return (
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
  );
}
