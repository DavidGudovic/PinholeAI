// Helper models (Describe / Improve): load the list, and save the choice for one purpose.
import { useEffect, useState } from "react";
import * as api from "./api";
import type { HelperModel, HelperPurpose } from "./types";
import { emitSettingsChanged } from "../settings/events";

/** Helper models from the registry (installed or not); null until loaded. Refreshes when models change. */
export function useHelperModels(): HelperModel[] | null {
  const [list, setList] = useState<HelperModel[] | null>(null);
  useEffect(() => {
    let alive = true;
    const load = () =>
      api
        .listHelperModels()
        .then((l) => alive && setList(l))
        .catch(() => undefined);
    void load();
    const un = api.onModelsChanged(load);
    const onSettings = () => void load();
    window.addEventListener("pinhole:settings-changed", onSettings);
    return () => {
      alive = false;
      void un.then((f) => f()).catch(() => undefined);
      window.removeEventListener("pinhole:settings-changed", onSettings);
    };
  }, []);
  return list;
}

/** Save which helper model a purpose uses ("auto" = Pinhole picks). */
export async function chooseHelper(purpose: HelperPurpose, id: string) {
  const current = await api.getSettings();
  const next = await api.setSettings({ ...current, [purpose === "improve" ? "improveModel" : "describeModel"]: id });
  emitSettingsChanged(next);
}
