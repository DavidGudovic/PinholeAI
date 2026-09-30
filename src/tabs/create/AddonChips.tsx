// Style add-ons in use, under the prompt (SPEC §5.1). Only shown while at least one is added,
// so the default view stays minimal; the full list (and Add) lives in Fine-tune.
// Each chip shows the trigger words it adds; its popover picks them and lets the user type their own.
import { Check, Puzzle, TriangleAlert, X } from "lucide-react";
import { useState } from "react";
import { Button, Popover, cx, focusRing, inputClass } from "../../components/ui";
import { asCoreError } from "../../lib/api";
import { useActions } from "../../lib/state/AppProvider";
import { loraCompatible, pickedTriggerWords } from "../../lib/state/model";
import { useAppState, useDispatch } from "../../lib/state/store";
import type { InstalledLora, InstalledModel, LoraUse } from "../../lib/types";

export function AddonChips({ model }: { model: InstalledModel | null }) {
  const used = useAppState((s) => s.create.loras);
  const loras = useAppState((s) => s.loras);
  const autoAdd = useAppState((s) => s.settings?.addTriggerWords ?? true);
  const dispatch = useDispatch();
  if (!used.length) return null;
  const setLoras = (list: LoraUse[]) => dispatch({ type: "patchCreate", patch: { loras: list } });
  const setWeight = (i: number, weight: number) => setLoras(used.map((x, j) => (j === i ? { ...x, weight } : x)));
  const setWords = (i: number, words: string[]) => setLoras(used.map((x, j) => (j === i ? { ...x, words } : x)));
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
        const picked = ok ? pickedTriggerWords(u, l, autoAdd) : [];
        return (
          <span
            key={u.loraId}
            className={cx(
              "inline-flex max-w-full items-center rounded-full border text-xs",
              ok ? "border-amber-300 bg-amber-50 text-amber-900 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-200" : "border-neutral-300 text-neutral-500 dark:border-neutral-700",
            )}
          >
            <Popover
              width={280}
              trigger={(p) => (
                <button {...p} type="button" className={cx("inline-flex min-w-0 items-center gap-1 rounded-full py-0.5 pr-1 pl-2.5", focusRing)} title={ok ? "Change strength" : undefined}>
                  {!ok && <TriangleAlert className="h-3 w-3 shrink-0" />}
                  <span className="max-w-40 truncate">{name}</span>
                  <span className="tabular-nums opacity-70">{u.weight.toFixed(2).replace(/0$/, "")}</span>
                  {picked.length > 0 && (
                    <span className="max-w-32 truncate opacity-70" title={`Adds to your prompt: ${picked.join(", ")}`}>
                      + {picked[0]}
                      {picked.length > 1 && ` +${picked.length - 1}`}
                    </span>
                  )}
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
                {l && ok && <TriggerWords lora={l} picked={picked} onPick={(words) => setWords(i, words)} />}
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

/** Trigger words of one add-on in use: tap to add or leave out; Edit to type the add-on's own list. */
function TriggerWords({ lora, picked, onPick }: { lora: InstalledLora; picked: string[]; onPick: (words: string[]) => void }) {
  const actions = useActions();
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const words = lora.trainedWords;
  const isPicked = (w: string) => picked.includes(w);
  const toggle = (w: string) => onPick(isPicked(w) ? picked.filter((p) => p !== w) : words.filter((x) => x === w || isPicked(x)));

  const save = async () => {
    if (draft == null) return;
    setBusy(true);
    setError(null);
    try {
      await actions.setLoraTriggerWords(lora.id, draft.split(",").map((w) => w.trim()).filter(Boolean));
      setDraft(null);
    } catch (e) {
      setError(asCoreError(e).message);
    } finally {
      setBusy(false);
    }
  };

  if (draft != null) {
    return (
      <div className="space-y-1.5">
        <label className="block text-xs text-neutral-600 dark:text-neutral-400">
          Trigger words
          <input
            className={cx(inputClass, "mt-1 h-8 text-xs")}
            value={draft}
            placeholder="e.g. watercolor style, ink wash"
            autoFocus
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void save();
            }}
          />
        </label>
        <p className="text-[11px] text-neutral-500">Separate words with commas. The add-on’s page usually lists them.</p>
        {error && <p className="text-[11px] text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-1.5">
          <Button size="sm" variant="ghost" onClick={() => setDraft(null)} disabled={busy}>
            Cancel
          </Button>
          <Button size="sm" variant="primary" onClick={() => void save()} disabled={busy}>
            Save
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between text-xs text-neutral-600 dark:text-neutral-400">
        <span>Trigger words</span>
        <button type="button" className={cx("rounded px-1 text-xs font-medium text-amber-700 hover:underline dark:text-amber-400", focusRing)} onClick={() => setDraft(words.join(", "))}>
          {words.length ? "Edit" : "Add"}
        </button>
      </div>
      {words.length ? (
        <>
          <div className="flex flex-wrap gap-1">
            {words.map((w, n) => (
              <button
                key={`${n}:${w}`}
                type="button"
                aria-pressed={isPicked(w)}
                onClick={() => toggle(w)}
                className={cx(
                  "inline-flex max-w-full items-center gap-1 rounded-full border px-2 py-0.5 text-[11px]",
                  focusRing,
                  isPicked(w)
                    ? "border-amber-400 bg-amber-100 text-amber-900 dark:border-amber-500/40 dark:bg-amber-500/20 dark:text-amber-100"
                    : "border-neutral-300 text-neutral-500 hover:border-neutral-400 dark:border-neutral-700",
                )}
              >
                {isPicked(w) && <Check className="h-3 w-3 shrink-0" />}
                <span className="truncate">{w}</span>
              </button>
            ))}
          </div>
          <p className="text-[11px] text-neutral-500">
            {picked.length ? "Ticked words are added to the end of your prompt when you make the picture (not twice if you typed them)." : "Tap a word to add it to your prompt when you make the picture."}
          </p>
        </>
      ) : (
        <p className="text-[11px] text-neutral-500">None saved. If the add-on’s page lists a trigger word, add it here so it works fully.</p>
      )}
    </div>
  );
}
