// The brush panel: "Only change here" (or Fix details' "Spot to fix"), paint or erase, brush size.
import { Brush, Eraser, Trash } from "lucide-react";
import { Button, Segmented, Slider, Toggle } from "../../components/ui";

export function MaskControls({
  fixing,
  maskOn,
  setMaskOn,
  painted,
  erase,
  setErase,
  brush,
  setBrush,
  onClear,
}: {
  fixing: boolean;
  maskOn: boolean;
  setMaskOn: (on: boolean) => void;
  painted: boolean;
  erase: boolean;
  setErase: (erase: boolean) => void;
  brush: number;
  setBrush: (size: number) => void;
  onClear: () => void;
}) {
  return (
    <div className="rounded-xl border border-neutral-200 p-3 dark:border-neutral-800">
      {fixing ? (
        <div>
          <div className="inline-flex items-center gap-1.5 text-sm font-medium">
            <Brush className="h-3.5 w-3.5" /> Spot to fix
          </div>
          <p className="mt-0.5 text-xs text-neutral-500">
            {painted
              ? "Paint a little past the edges so it blends in."
              : "Paint over the face, hand or detail to redraw. Paint nothing to add detail to every face."}
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
              onClick={onClear}
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
  );
}
