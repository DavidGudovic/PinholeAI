// Save (with Save as…) and Upscale buttons shared by Create's results and the Edit tab.
import { useState } from "react";
import { ChevronDown, ImageUp, Save } from "lucide-react";
import { useShortcuts } from "../lib/shortcuts";
import { useActions } from "../lib/state/AppProvider";
import { willQueue, type TabId } from "../lib/state/model";
import { useAppState } from "../lib/state/store";
import { canSaveAs } from "../lib/state/platform";
import { Button, MenuItem, MenuSeparator, Popover, cx, focusRing } from "./ui";

/** Largest side the upscaler can output (it works at 4× first). Mirrors upscale_image in generate.rs. */
export const UPSCALE_MAX_SIDE = 8192;

/** Save, plus "Save as…" (and, with `onSaveAll`, "Save all unsaved"; with `onSaveSheet`, "Save as one sheet") in the desktop app. Errors go to `run`. */
export function SaveButton({
  id,
  seed,
  size = "md",
  run,
  tab,
  unsavedCount = 0,
  onSaveAll,
  sheetCount = 0,
  onSaveSheet,
}: {
  id: string;
  seed: number | null;
  size?: "sm" | "md";
  /** The tab it sits in: S and Ctrl/Cmd+Shift+S save this image while that tab is showing. */
  tab: TabId;
  run: (f: () => Promise<unknown>) => Promise<void>;
  /** Unsaved pictures in the session; "Save all unsaved" shows when there are 2 or more. */
  unsavedCount?: number;
  onSaveAll?: () => void;
  /** Pictures that came with this one; "Save as one sheet" shows when there are 2 or more. */
  sheetCount?: number;
  onSaveSheet?: () => void;
}) {
  const actions = useActions();
  const [saving, setSaving] = useState(false);
  const icon = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";
  const save = () => {
    // The toast from actions.save confirms it; a double-click must not write two files.
    if (saving) return;
    setSaving(true);
    void run(() => actions.save(id)).finally(() => setSaving(false));
  };
  useShortcuts(tab, {
    save,
    saveAs: canSaveAs() ? () => void run(() => actions.saveAs(id, seed)) : undefined,
  });
  return (
    <div className="inline-flex">
      <Button
        variant="secondary"
        size={size}
        className={cx(canSaveAs() && "rounded-r-none")}
        disabled={saving}
        onClick={save}
      >
        <Save className={icon} /> Save
      </Button>
      {canSaveAs() && (
        <Popover
          align="end"
          width={230}
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
            <>
              <MenuItem
                hint="Choose the name and folder"
                onClick={() => {
                  close();
                  void run(() => actions.saveAs(id, seed));
                }}
              >
                Save as…
              </MenuItem>
              {onSaveAll && unsavedCount > 1 && (
                <MenuItem
                  hint="Every picture not saved yet, into a folder you choose"
                  onClick={() => {
                    close();
                    onSaveAll();
                  }}
                >
                  Save all unsaved ({unsavedCount})
                </MenuItem>
              )}
              {onSaveSheet && sheetCount > 1 && (
                <MenuItem
                  hint="The pictures made with this one, side by side in one picture"
                  onClick={() => {
                    close();
                    onSaveSheet();
                  }}
                >
                  Save as one sheet ({sheetCount})
                </MenuItem>
              )}
            </>
          )}
        </Popover>
      )}
    </div>
  );
}

/** "Upscale ▾" with 2× and 4×. Picked while a job runs, it waits in the queue. */
export function UpscaleMenu({
  width,
  height,
  disabled,
  size = "md",
  onPick,
  onFinish,
}: {
  width: number;
  height: number;
  disabled?: boolean;
  size?: "sm" | "md";
  onPick: (factor: 2 | 4) => void;
  /** "Finish at Best quality" on top (Create pictures made below Best). */
  onFinish?: () => void;
}) {
  const queues = useAppState(willQueue);
  // The upscaler always runs at 4× first (2× is 4× halved), up to 8192 px per side.
  const tooBig = width * 4 > UPSCALE_MAX_SIDE || height * 4 > UPSCALE_MAX_SIDE;
  const icon = size === "sm" ? "h-3.5 w-3.5" : "h-4 w-4";
  return (
    <Popover
      width={onFinish ? 250 : 200}
      trigger={(p) => (
        <Button {...p} size={size} disabled={disabled}>
          <ImageUp className={icon} /> Upscale{" "}
          <ChevronDown className="h-3.5 w-3.5 opacity-60" />
        </Button>
      )}
    >
      {(close) => (
        <>
          {onFinish && (
            <>
              <MenuItem
                hint={`Same picture, more detail${queues ? " (waits for the current job)" : ""}`}
                onClick={() => {
                  close();
                  onFinish();
                }}
              >
                Finish at Best quality
              </MenuItem>
              <MenuSeparator />
            </>
          )}
          {([2, 4] as const).map((f) => (
            <MenuItem
              key={f}
              disabled={tooBig}
              hint={
                tooBig
                  ? `Too large to upscale (max ${UPSCALE_MAX_SIDE / 4} px per side)`
                  : `${width * f}×${height * f}${queues ? " (waits for the current job)" : ""}`
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
