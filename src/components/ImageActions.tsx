// Save (with Save as…) and Upscale buttons shared by Create's results and the Edit tab.
import { useState } from "react";
import { ChevronDown, ImageUp, Save } from "lucide-react";
import { useActions } from "../lib/state/AppProvider";
import { canSaveAs } from "../lib/state/platform";
import { Button, MenuItem, Popover, cx, focusRing } from "./ui";

/** Largest side the upscaler can output (it works at 4× first). Mirrors upscale_image in generate.rs. */
export const UPSCALE_MAX_SIDE = 8192;

/** Save, plus "Save as…" in the desktop app. Errors go to `run`. */
export function SaveButton({
  id,
  seed,
  size = "md",
  run,
}: {
  id: string;
  seed: number | null;
  size?: "sm" | "md";
  run: (f: () => Promise<unknown>) => Promise<void>;
}) {
  const actions = useActions();
  const [saving, setSaving] = useState(false);
  const icon = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";
  return (
    <div className="inline-flex">
      <Button
        variant="secondary"
        size={size}
        className={cx(canSaveAs() && "rounded-r-none")}
        disabled={saving}
        onClick={() => {
          // The toast from actions.save confirms it; a double-click must not write two files.
          if (saving) return;
          setSaving(true);
          void run(() => actions.save(id)).finally(() => setSaving(false));
        }}
      >
        <Save className={icon} /> Save
      </Button>
      {canSaveAs() && (
        <Popover
          align="end"
          width={180}
          trigger={(p) => (
            <button
              {...p}
              type="button"
              aria-label="More save options"
              className={cx(
                "inline-flex items-center rounded-r-lg border border-l-0 border-neutral-200 bg-white px-1.5 hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:hover:bg-neutral-700",
                size === "sm" ? "h-7" : "h-9",
                focusRing,
              )}
            >
              <ChevronDown className={icon} />
            </button>
          )}
        >
          {(close) => (
            <MenuItem
              onClick={() => {
                close();
                void run(() => actions.saveAs(id, seed));
              }}
            >
              Save as…
            </MenuItem>
          )}
        </Popover>
      )}
    </div>
  );
}

/** "Upscale ▾" with 2× and 4×. */
export function UpscaleMenu({
  width,
  height,
  disabled,
  size = "md",
  onPick,
}: {
  width: number;
  height: number;
  disabled?: boolean;
  size?: "sm" | "md";
  onPick: (factor: 2 | 4) => void;
}) {
  // The upscaler always runs at 4× first (2× is 4× halved), up to 8192 px per side.
  const tooBig = width * 4 > UPSCALE_MAX_SIDE || height * 4 > UPSCALE_MAX_SIDE;
  const icon = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";
  return (
    <Popover
      width={200}
      trigger={(p) => (
        <Button {...p} size={size} disabled={disabled}>
          <ImageUp className={icon} /> Upscale{" "}
          <ChevronDown className="h-3.5 w-3.5 opacity-60" />
        </Button>
      )}
    >
      {(close) => (
        <>
          {([2, 4] as const).map((f) => (
            <MenuItem
              key={f}
              disabled={tooBig}
              hint={
                tooBig
                  ? `Too large to upscale (max ${UPSCALE_MAX_SIDE / 4} px per side)`
                  : `${width * f}×${height * f}`
              }
              onClick={() => {
                close();
                onPick(f);
              }}
            >
              Upscale {f}×
            </MenuItem>
          ))}
        </>
      )}
    </Popover>
  );
}
