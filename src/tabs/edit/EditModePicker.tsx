// Edit's mode picker (Describe a change, Restyle, Fix details, Extend) and the hint under it.
import { Segmented } from "../../components/ui";
import type { InstalledModel } from "../../lib/types";
import type { Action, EditMode, EditParams } from "../../lib/state/model";

export function EditModePicker({
  e,
  mode,
  autoEdit,
  dispatch,
}: {
  e: EditParams;
  mode: EditMode;
  autoEdit: InstalledModel | null;
  dispatch: (a: Action) => void;
}) {
  const fixing = mode === "fix";
  const extending = mode === "extend";
  return (
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
              ? "Paint over a small spot, like a face or hand, to redraw it larger and blend it back in. Paint nothing to add detail to every face."
              : extending
                ? "Pick a new shape. Pinhole adds space around your picture and draws what fits there."
                : "Redraws the whole picture with your description."}
      </p>
    </div>
  );
}
