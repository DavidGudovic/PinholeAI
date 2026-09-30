// Edit → Extend (SPEC §5.2): pick the new shape and where the new space goes.
import { Segmented } from "../../components/ui";
import { DEFAULT_SHAPE_SIZES, SHAPE_LABEL } from "../../lib/paste/map";
import type { FamilyUi, Shape } from "../../lib/types";
import type { ExtendSide, ExtendTo } from "../../lib/state/model";
import { extendCanvas } from "../../lib/state/request";
import { ShapeChip } from "../create/Dials";

const SHAPES: Shape[] = ["square", "portrait", "landscape", "wide"];

export function ExtendControls({
  to,
  side,
  width,
  height,
  ui,
  onChange,
}: {
  to: ExtendTo;
  side: ExtendSide;
  /** The picture being extended. */
  width: number;
  height: number;
  ui: FamilyUi | null;
  onChange: (patch: { extendTo?: ExtendTo; extendSide?: ExtendSide }) => void;
}) {
  const canvas = extendCanvas(width, height, to, side, ui);
  const wider = !!canvas && canvas.width > width;
  return (
    <div className="space-y-3">
      <div>
        <div className="mb-1.5 text-sm text-neutral-600 dark:text-neutral-400">New shape</div>
        <div role="radiogroup" aria-label="New shape" className="grid grid-cols-5 gap-1.5">
          {SHAPES.map((s) => {
            const [w, h] = ui?.shapes[s] ?? DEFAULT_SHAPE_SIZES[s];
            const same = !extendCanvas(width, height, s, side, ui);
            return (
              <ShapeChip
                key={s}
                active={s === to}
                w={w}
                h={h}
                label={SHAPE_LABEL[s]}
                title={same ? "The picture is already this shape" : `Make it ${SHAPE_LABEL[s].toLowerCase()}`}
                disabled={same && s !== to}
                onClick={() => onChange({ extendTo: s })}
              />
            );
          })}
          <ShapeChip
            active={to === "around"}
            w={width}
            h={height}
            inset
            label="Around"
            title="Same shape, with more space on every side"
            onClick={() => onChange({ extendTo: "around" })}
          />
        </div>
      </div>
      {to !== "around" && canvas && (
        <div>
          <div className="mb-1.5 text-sm text-neutral-600 dark:text-neutral-400">Add the space</div>
          <Segmented
            stretch
            size="sm"
            ariaLabel="Add the space"
            value={side}
            onChange={(v) => onChange({ extendSide: v })}
            options={[
              { value: "start" as ExtendSide, label: wider ? "Left" : "Top" },
              { value: "both" as ExtendSide, label: "Both sides" },
              { value: "end" as ExtendSide, label: wider ? "Right" : "Bottom" },
            ]}
          />
        </div>
      )}
      <p className="text-xs text-neutral-500 tabular-nums">
        {canvas
          ? `${width}×${height} → ${canvas.width}×${canvas.height}. The new space is drawn to match; your picture stays as it is, blended in at the edge.`
          : "The picture is already this shape. Pick another one."}
      </p>
    </div>
  );
}
