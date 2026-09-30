// Style add-ons in use, under the prompt (SPEC §5.1). Only shown while at least one is added,
// so the default view stays minimal; the full list (and Add) lives in Fine-tune.
import { Puzzle, TriangleAlert, X } from "lucide-react";
import { Popover, cx, focusRing } from "../../components/ui";
import { loraCompatible } from "../../lib/state/model";
import { useAppState, useDispatch } from "../../lib/state/store";
import type { InstalledModel, LoraUse } from "../../lib/types";

export function AddonChips({ model }: { model: InstalledModel | null }) {
  const used = useAppState((s) => s.create.loras);
  const loras = useAppState((s) => s.loras);
  const dispatch = useDispatch();
  if (!used.length) return null;
  const setLoras = (list: LoraUse[]) => dispatch({ type: "patchCreate", patch: { loras: list } });
  const setWeight = (i: number, weight: number) => setLoras(used.map((x, j) => (j === i ? { ...x, weight } : x)));
  const remove = (i: number) => setLoras(used.filter((_, j) => j !== i));

  return (
    <div role="group" aria-label="Style add-ons" className="flex flex-wrap items-center gap-1.5">
      <span className="mr-0.5 inline-flex items-center gap-1 text-xs font-medium text-neutral-500 dark:text-neutral-400">
        <Puzzle className="h-3.5 w-3.5" /> Add-ons
      </span>
      {used.map((u, i) => {
        const l = loras.find((x) => x.id === u.loraId);
        const name = l?.friendlyName ?? "Missing add-on";
        const ok = !!l && loraCompatible(l, model?.familyId);
        return (
          <span
            key={u.loraId}
            className={cx(
              "inline-flex max-w-full items-center rounded-full border text-xs",
              ok ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200" : "border-neutral-300 text-neutral-500 dark:border-neutral-700",
            )}
          >
            <Popover
              width={260}
              trigger={(p) => (
                <button {...p} type="button" className={cx("inline-flex min-w-0 items-center gap-1 rounded-full py-0.5 pr-1 pl-2.5", focusRing)} title={ok ? "Change strength" : undefined}>
                  {!ok && <TriangleAlert className="h-3 w-3 shrink-0" />}
                  <span className="max-w-40 truncate">{name}</span>
                  <span className="tabular-nums opacity-70">{u.weight.toFixed(2).replace(/0$/, "")}</span>
                </button>
              )}
            >
              <div className="space-y-2 p-2.5">
                <div className="truncate text-sm font-medium">{name}</div>
                <label className="block text-xs text-neutral-600 dark:text-neutral-400">
                  <span className="flex justify-between">
                    <span>Strength</span>
                    <span className="tabular-nums">{u.weight.toFixed(2)}</span>
                  </span>
                  <input
                    type="range"
                    aria-label={`Strength of ${name}`}
                    className="pinhole-range mt-1 w-full"
                    min={0}
                    max={1.5}
                    step={0.05}
                    value={Math.min(1.5, Math.max(0, u.weight))}
                    onChange={(e) => setWeight(i, Number(e.target.value))}
                  />
                </label>
                {l && l.trainedWords.length > 0 && <p className="text-[11px] text-neutral-500">Trigger words: {l.trainedWords.join(", ")}</p>}
                {!ok && (
                  <p className="text-[11px] text-amber-700 dark:text-amber-400">
                    {l ? `Made for ${l.baseModel ?? "other"} models, so it isn't used with this one.` : "This add-on isn't installed any more."}
                  </p>
                )}
              </div>
            </Popover>
            <button
              type="button"
              aria-label={`Remove ${name}`}
              title={`Remove ${name}`}
              className={cx("mr-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-full hover:bg-black/10 dark:hover:bg-white/10", focusRing)}
              onClick={() => remove(i)}
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        );
      })}
    </div>
  );
}
