// Fine-tune control for which upscaler Upscale uses. Saved in Settings (`upscaler`), so Create
// and Edit share it. Automatic: the photo upscaler for photo-style pictures, the drawing
// upscaler for the rest.
import { useState } from "react";
import { Segmented } from "./ui";
import { asCoreError } from "../lib/api";
import { chooseUpscaler } from "../lib/helpers";
import { useAppState, useDispatch } from "../lib/state/store";
import type { UpscalerChoice } from "../lib/types";

export const UPSCALER_HINT = "Automatic uses the photo upscaler for photo-style pictures and the drawing upscaler for the rest.";

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

export function UpscalerSegmented({ upscaler }: { upscaler: ReturnType<typeof useUpscaler> }) {
  return (
    <div className="flex flex-col gap-1">
      <Segmented
        size="sm"
        ariaLabel="Upscaler"
        value={upscaler.value}
        onChange={upscaler.choose}
        options={[
          { value: "auto" as UpscalerChoice, label: "Auto", title: UPSCALER_HINT },
          { value: "photo" as UpscalerChoice, label: "Photo" },
          { value: "drawing" as UpscalerChoice, label: "Drawing" },
        ]}
      />
      {upscaler.error && <p className="text-xs text-red-600">{upscaler.error}</p>}
    </div>
  );
}
