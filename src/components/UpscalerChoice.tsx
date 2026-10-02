// Fine-tune control for which upscaler Upscale uses. Saved in Settings (`upscaler`), so Create
// and Edit share it. Automatic: the smooth photo upscaler for photo-style pictures, the drawing
// upscaler for the rest.
import { useState } from "react";
import { Select } from "./ui";
import { asCoreError } from "../lib/api";
import { chooseUpscaler } from "../lib/helpers";
import { useAppState, useDispatch } from "../lib/state/store";
import type { UpscalerChoice } from "../lib/types";

/** One plain line per option: what it is good at and what it can get wrong. */
export const UPSCALER_OPTIONS: { value: UpscalerChoice; label: string }[] = [
  { value: "auto", label: "Auto: picks by picture style" },
  { value: "photo", label: "Photo: smooth (good for hair, can look waxy)" },
  { value: "photo_texture", label: "Photo: skin texture (real skin, can make beards crunchy)" },
  { value: "drawing", label: "Drawing: clean lines and flat colour" },
];

export const UPSCALER_HINT = "Auto uses Photo: smooth for photo-style pictures and Drawing for the rest. Each upscaler downloads once, the first time it is used.";

/** The saved choice, and a function that saves a new one. */
export function useUpscaler(): { value: UpscalerChoice; choose: (c: UpscalerChoice) => void; error: string | null } {
  const value = useAppState((s) => s.settings?.upscaler) ?? "auto";
  const dispatch = useDispatch();
  const [error, setError] = useState<string | null>(null);
  const choose = (choice: UpscalerChoice) => {
    setError(null);
    chooseUpscaler(choice)
      .then((settings) => dispatch({ type: "setSettings", settings }))
      .catch((e) => setError(asCoreError(e).message));
  };
  return { value, choose, error };
}

export function UpscalerSelect({ upscaler }: { upscaler: ReturnType<typeof useUpscaler> }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <Select ariaLabel="Upscaler" value={upscaler.value} onChange={(v) => upscaler.choose(v as UpscalerChoice)}>
        {UPSCALER_OPTIONS.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </Select>
      <p className="text-[11px] text-neutral-500">{UPSCALER_HINT}</p>
      {upscaler.error && <p className="text-xs text-red-600">{upscaler.error}</p>}
    </div>
  );
}

