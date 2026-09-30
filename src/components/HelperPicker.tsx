// A small picker for the helper model behind Describe or "Improve my prompt".
// Automatic = Pinhole picks (the larger model when it is already installed). Only installed helpers
// can be chosen; "Get more models…" opens Models → Helpers.
import { useState } from "react";
import { Select } from "./ui";
import { asCoreError } from "../lib/api";
import { chooseHelper, useHelperModels } from "../lib/helpers";
import { useActions } from "../lib/state/AppProvider";
import { useAppState } from "../lib/state/store";
import { requestHelpersView } from "../tabs/models/lib/session";
import type { HelperPurpose } from "../lib/types";

const MORE = "__more";

export function HelperPicker({ purpose, className }: { purpose: HelperPurpose; className?: string }) {
  const models = useHelperModels();
  const actions = useActions();
  const picked = useAppState((s) => (purpose === "improve" ? s.settings?.improveModel : s.settings?.describeModel)) ?? "auto";
  const [error, setError] = useState<string | null>(null);
  const installed = (models ?? []).filter((m) => m.installed);
  // A choice that isn't installed any more reads as Automatic (as the backend does).
  const value = installed.some((m) => m.id === picked) ? picked : "auto";
  if (!models) return null;

  return (
    <div className={className}>
      <Select
        ariaLabel={purpose === "improve" ? "Model for Improve" : "Model for Describe"}
        value={value}
        className="w-44"
        onChange={(v) => {
          if (v === MORE) {
            requestHelpersView();
            actions.setTab("models");
            return;
          }
          setError(null);
          void chooseHelper(purpose, v).catch((e) => setError(asCoreError(e).message));
        }}
      >
        <option value="auto">Model: Automatic</option>
        {installed.map((m) => (
          <option key={m.id} value={m.id}>
            {m.title}
          </option>
        ))}
        <option value={MORE}>Get more models…</option>
      </Select>
      {error && <p className="mt-1 text-xs text-red-600 dark:text-red-400">{error}</p>}
    </div>
  );
}
