import { Check, ChevronDown, CircleAlert, Layers } from "lucide-react";
import type { InstalledModel } from "../lib/types";
import { useActions } from "../lib/state/AppProvider";
import { Badge, MenuItem, MenuSeparator, Popover, VramBadge, cx, focusRing } from "./ui";

const BADGE_TONE: Record<string, "blue" | "amber" | "green" | "neutral"> = {
  Realistic: "blue",
  Anime: "amber",
  Illustration: "green",
  "3D": "neutral",
};

export function StyleBadge({ badge }: { badge: string | null }) {
  if (!badge) return null;
  return <Badge tone={BADGE_TONE[badge] ?? "neutral"}>{badge}</Badge>;
}

/** Installed-model picker: friendly name, style badge, VRAM need + fit. */
export function ModelPicker({
  models,
  value,
  onChange,
  label = "Model",
  hint,
  emptyText = "No models installed",
}: {
  models: InstalledModel[];
  value: string | null;
  onChange: (id: string) => void;
  label?: string;
  hint?: string;
  emptyText?: string;
}) {
  const actions = useActions();
  const current = models.find((m) => m.id === value) ?? null;
  return (
    <Popover
      width="trigger"
      className="min-w-80"
      trigger={(p) => (
        <button
          {...p}
          type="button"
          aria-label={`${label}: ${current?.friendlyName ?? "none"}`}
          className={cx(
            "group flex w-full items-center gap-3 rounded-xl border border-neutral-200 bg-white px-3 py-2 text-left shadow-xs transition-colors hover:border-neutral-300 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:border-neutral-700",
            focusRing,
          )}
        >
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-neutral-100 text-neutral-500 dark:bg-neutral-800">
            <Layers className="h-4 w-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1.5 text-[11px] font-medium text-neutral-500">
              {hint ?? label}
              {current && <StyleBadge badge={current.styleBadge} />}
            </span>
            <span className="block truncate text-sm font-semibold">{current?.friendlyName ?? emptyText}</span>
            {current?.vram && (
              <span className="mt-0.5 block">
                <VramBadge vram={current.vram} fit={current.fit} />
              </span>
            )}
          </span>
          <ChevronDown className="h-4 w-4 shrink-0 text-neutral-400 transition-transform group-aria-expanded:rotate-180" />
        </button>
      )}
    >
      {(close) => (
        <div role="listbox" aria-label={label}>
          {models.map((m) => (
            <MenuItem
              key={m.id}
              selected={m.id === value}
              onClick={() => {
                onChange(m.id);
                close();
              }}
              right={m.id === value ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
              hint={
                <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                  <span>{m.familyLabel ?? "Unknown family"}</span>
                  <VramBadge vram={m.vram} fit={m.fit} />
                  {m.missingComponents.length > 0 && (
                    <span className="inline-flex items-center gap-1 text-amber-700 dark:text-amber-400">
                      <CircleAlert className="h-3 w-3" /> Needs {m.missingComponents.join(", ")}
                    </span>
                  )}
                </span>
              }
            >
              <span className="flex items-center gap-2">
                <span className="truncate font-medium">{m.friendlyName}</span>
                <StyleBadge badge={m.styleBadge} />
              </span>
            </MenuItem>
          ))}
          {models.length > 0 && <MenuSeparator />}
          <MenuItem
            icon={<Layers className="h-4 w-4" />}
            onClick={() => {
              close();
              actions.setTab("models");
            }}
          >
            Get more models…
          </MenuItem>
        </div>
      )}
    </Popover>
  );
}
