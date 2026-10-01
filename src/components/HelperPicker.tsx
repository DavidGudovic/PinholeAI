// The model picker for Describe: the same card as Create's model picker, listing the helper models
// that can describe pictures. It reads and saves the same choice as Settings → Helper models
// ("Automatic" = Pinhole picks, the larger model when it is installed). Only installed helpers can be
// chosen; "Get more models…" opens Models → Helpers. Safe-mode-Off helpers never show while Safe mode
// is On (the backend leaves them out of the list too).
import { useState } from "react";
import { Check, ChevronDown, ScanText } from "lucide-react";
import { Badge, MenuItem, MenuSeparator, Popover, cx, focusRing } from "./ui";
import { asCoreError } from "../lib/api";
import { chooseHelper, useHelperModels } from "../lib/helpers";
import { useActions } from "../lib/state/AppProvider";
import { useAppState } from "../lib/state/store";
import { FIT_WORDS } from "../tabs/models/HelpersView";
import { requestHelpersView } from "../tabs/models/lib/session";
import type { HelperModel, HelperPurpose } from "../lib/types";

function FitBadge({ m }: { m: HelperModel }) {
  if (!m.fit) return null;
  const w = FIT_WORDS[m.fit];
  return (
    <span title={w.title}>
      <Badge tone={w.tone}>{w.label}</Badge>
    </span>
  );
}

export function HelperPicker({ purpose, label = "Model" }: { purpose: HelperPurpose; label?: string }) {
  const models = useHelperModels();
  const actions = useActions();
  const picked = useAppState((s) => (purpose === "improve" ? s.settings?.improveModel : s.settings?.describeModel)) ?? "auto";
  const safe = useAppState((s) => (s.settings?.contentMode ?? "safe") !== "all");
  const [error, setError] = useState<string | null>(null);
  if (!models) return null;
  const installed = models.filter((m) => m.installed && !(safe && m.needsSafeOff));
  // A choice that isn't installed (or offered) any more reads as Automatic, as the backend does.
  const current = installed.find((m) => m.id === picked) ?? null;
  const choose = (id: string) => {
    setError(null);
    void chooseHelper(purpose, id).catch((e) => setError(asCoreError(e).message));
  };

  return (
    <div>
      <Popover
        width="trigger"
        className="min-w-72"
        trigger={(p) => (
          <button
            {...p}
            type="button"
            aria-label={`${label}: ${current?.title ?? "Automatic"}`}
            className={cx(
              "group flex w-full items-center gap-3 rounded-xl border border-neutral-200 bg-white px-3 py-2 text-left shadow-xs transition-colors hover:border-neutral-300 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:border-neutral-700",
              focusRing,
            )}
          >
            <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-neutral-500 dark:bg-neutral-800">
              <ScanText className="h-4 w-4" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-500">
                {label}
                {current && <FitBadge m={current} />}
              </span>
              <span className="block truncate text-sm font-semibold">{current?.title ?? "Automatic"}</span>
            </span>
            <ChevronDown className="h-4 w-4 shrink-0 text-neutral-400 transition-transform group-aria-expanded:rotate-180" />
          </button>
        )}
      >
        {(close) => (
          <div role="listbox" aria-label={label}>
            <MenuItem
              selected={!current}
              onClick={() => {
                choose("auto");
                close();
              }}
              right={!current ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
              hint="Pinhole picks. Uses the larger model when it is installed."
            >
              <span className="font-medium">Automatic</span>
            </MenuItem>
            {installed.map((m) => (
              <MenuItem
                key={m.id}
                selected={m.id === current?.id}
                onClick={() => {
                  choose(m.id);
                  close();
                }}
                right={m.id === current?.id ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
                hint={<FitBadge m={m} />}
              >
                <span className="truncate font-medium">{m.title}</span>
              </MenuItem>
            ))}
            <MenuSeparator />
            <MenuItem
              icon={<ScanText className="h-4 w-4" />}
              onClick={() => {
                close();
                requestHelpersView();
                actions.setTab("models");
              }}
            >
              Get more models…
            </MenuItem>
          </div>
        )}
      </Popover>
      {error && <p className="mt-1 text-xs text-red-600 dark:text-red-400">{error}</p>}
    </div>
  );
}
