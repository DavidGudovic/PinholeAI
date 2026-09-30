// One quiet "Tip" line under a picture: a feature people often miss. At most one per app session
// (it stays until dismissed), and "Don't show tips" turns it off for good (Settings → Show tips).
import { useState } from "react";
import { Lightbulb, X } from "lucide-react";
import * as api from "../../lib/api";
import { takesReference } from "../../lib/state/model";
import { useAppState, useDispatch } from "../../lib/state/store";
import { cx, focusRing } from "../../components/ui";

export type TipId = "keep" | "variations" | "styles" | "paste" | "reference" | "shortcuts";

export const TIPS: Record<TipId, string> = {
  keep: "Like this one? Turn on “Keep this look” under the dials to reuse its starting point while you change the prompt.",
  variations: "Variations makes more pictures from the same prompt, each with a new random start.",
  styles: "Wording you use a lot? Save it as a Style next to the prompt box and pick it any time.",
  paste: "Found a picture you like on CivitAI? “Paste from CivitAI” fills in its prompt and settings.",
  reference: "This model can follow a reference picture. Drop one under the prompt, then say how to use it.",
  shortcuts: "Press ? any time to see the keyboard shortcuts.",
};

/** Tips that make sense right now, in the order they are offered. */
export function eligibleTips(o: { hasBatch: boolean; canReference: boolean }): TipId[] {
  const out: TipId[] = ["keep"];
  if (o.hasBatch) out.push("variations");
  out.push("styles", "paste");
  if (o.canReference) out.push("reference");
  out.push("shortcuts");
  return out;
}

// The session's tip: chosen the first time one is shown, and gone once closed.
let sessionTip: TipId | null = null;
let closedThisSession = false;
/** Tests only. */
export function resetSessionTip() {
  sessionTip = null;
  closedThisSession = false;
}

export function TipLine({ hasBatch }: { hasBatch: boolean }) {
  const show = useAppState((s) => s.settings?.showTips ?? true);
  const canReference = useAppState((s) => takesReference(s.models?.find((m) => m.id === s.create.modelId)));
  const dispatch = useDispatch();
  const [, rerender] = useState(0);
  if (!show || closedThisSession) return null;
  // Picked once per session; picked again only if it stopped making sense (another model, a single picture).
  const ids = eligibleTips({ hasBatch, canReference });
  if (!sessionTip || !ids.includes(sessionTip)) sessionTip = ids[Math.floor(Math.random() * ids.length)];
  const close = () => {
    closedThisSession = true;
    rerender((n) => n + 1);
  };
  const turnOff = () => {
    close();
    void api
      .getSettings()
      .then((s) => api.setSettings({ ...s, showTips: false }))
      .then((s) => dispatch({ type: "setSettings", settings: s }))
      .catch(() => undefined);
  };
  return (
    <p className="mx-auto flex max-w-xl items-start gap-2 text-xs text-neutral-500" role="note">
      <Lightbulb className="mt-px h-3.5 w-3.5 shrink-0 text-amber-500" aria-hidden />
      <span className="min-w-0">
        <span className="font-medium text-neutral-600 dark:text-neutral-300">Tip:</span> {TIPS[sessionTip]}{" "}
        <button type="button" onClick={turnOff} className={cx("rounded underline underline-offset-2 hover:text-neutral-900 dark:hover:text-white", focusRing)}>
          Don’t show tips
        </button>
      </span>
      <button type="button" aria-label="Dismiss tip" onClick={close} className={cx("shrink-0 rounded p-0.5 hover:text-neutral-900 dark:hover:text-white", focusRing)}>
        <X className="h-3.5 w-3.5" />
      </button>
    </p>
  );
}
