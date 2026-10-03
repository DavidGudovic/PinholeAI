// A picture pasted with Ctrl/Cmd+V anywhere in the app, or dropped where no drop area takes it:
// ask what it's for (Create's reference picture, Edit or Describe), then import it the same way
// as a chosen file.
import { useState } from "react";
import { ImagePlus, ScanText, WandSparkles } from "lucide-react";
import * as api from "../lib/api";
import { useActions } from "../lib/state/AppProvider";
import { editBusy, takesReference, unsavedEditIds, type TabId } from "../lib/state/model";
import { useAppState, useStore } from "../lib/state/store";
import type { CoreError } from "../lib/types";
import { useImageDrop, useImagePaste, useOfferedPicture } from "./ImageDrop";
import { Dialog, ErrorNotice, Spinner, cx, focusRing } from "./ui";

export type PasteTarget = "reference" | "edit" | "describe";

/** The choices for a pasted picture. The reference picture only while Create shows its slot. */
export function pasteTargets(referenceShown: boolean): PasteTarget[] {
  return referenceShown ? ["reference", "edit", "describe"] : ["edit", "describe"];
}

/** The choice focused first: the one for the tab you're on. */
export function defaultPasteTarget(tab: TabId, targets: PasteTarget[]): PasteTarget {
  const mine: PasteTarget | null = tab === "create" ? "reference" : tab === "edit" ? "edit" : tab === "describe" ? "describe" : null;
  return mine && targets.includes(mine) ? mine : targets.includes("edit") ? "edit" : targets[0];
}

const LABEL: Record<PasteTarget, string> = {
  reference: "Create reference picture",
  edit: "Edit",
  describe: "Describe",
};
const HINT: Record<PasteTarget, string> = {
  reference: "Make something in its style, or with the same character",
  edit: "Change it with words, fix a spot, extend or upscale it",
  describe: "Get a prompt or tags for it",
};
const ICON: Record<PasteTarget, typeof ImagePlus> = { reference: ImagePlus, edit: WandSparkles, describe: ScanText };

export function PasteChooser() {
  const tab = useAppState((s) => s.tab);
  const referenceShown = useAppState((s) => !!s.create.refImageId || takesReference((s.models ?? []).find((m) => m.id === s.create.modelId)));
  const actions = useActions();
  const store = useStore();
  const [file, setFile] = useState<File | null>(null);
  const [how, setHow] = useState<"pasted" | "dropped">("pasted");
  const [busy, setBusy] = useState<PasteTarget | null>(null);
  const [error, setError] = useState<CoreError | null>(null);

  const offer = (f: File, h: "pasted" | "dropped") => {
    setError(null);
    setHow(h);
    setFile(f);
  };
  useImagePaste(true, (f) => offer(f, "pasted"));
  useImageDrop(
    (f) => offer(f, "dropped"),
    (d) => {
      if (d.kind === "not-a-picture") actions.toast("That file isn’t a picture Pinhole can open. Try a PNG, JPEG or WebP.", { ms: 4500 });
      else if (d.kind === "link") actions.toast("Only the picture’s web address came through. In the browser, right-click the picture, choose Copy image, then paste it here.", { ms: 5000 });
    },
  );
  useOfferedPicture((f) => {
    if (!busy) offer(f, "dropped");
  });

  const targets = pasteTargets(referenceShown);
  const first = defaultPasteTarget(tab, targets);
  const close = () => {
    if (busy) return;
    setFile(null);
    setError(null);
  };

  const use = async (target: PasteTarget) => {
    if (!file || busy) return;
    if (target === "edit") {
      const s = store.getState();
      if (!editBusy(s) && unsavedEditIds(s).length) {
        // Edit first asks about its unsaved pictures, and opens itself if the user goes ahead.
        setFile(null);
        void actions.importToEdit(file).catch(() => undefined);
        return;
      }
    }
    setBusy(target);
    setError(null);
    try {
      if (target === "edit") {
        if (await actions.importToEdit(file)) actions.setTab("edit");
      } else if (target === "describe") {
        await actions.importToDescribe(file);
        actions.setTab("describe");
      } else {
        await actions.importCreateReference(file);
        actions.setTab("create");
      }
      setFile(null);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog open={!!file} onClose={close} title={`Use the ${how} picture for…`}>
      <div className="grid gap-2">
        {targets.map((t) => {
          const Icon = ICON[t];
          return (
            <button
              key={t}
              type="button"
              className={cx(
                "flex w-full items-center gap-3 rounded-lg border px-3.5 py-2.5 text-left text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-60",
                focusRing,
                t === first
                  ? "border-amber-500 bg-amber-50 text-neutral-900 hover:bg-amber-100 dark:bg-amber-500/10 dark:text-neutral-100 dark:hover:bg-amber-500/20"
                  : "border-neutral-200 bg-white text-neutral-800 hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:hover:bg-neutral-700",
              )}
              disabled={!!busy}
              data-autofocus={t === first ? "" : undefined}
              onClick={() => void use(t)}
            >
              {busy === t ? <Spinner className="h-4 w-4 shrink-0" /> : <Icon className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400" />}
              <span className="min-w-0">
                <span className="block font-medium">{LABEL[t]}</span>
                <span className="block text-xs text-neutral-500 dark:text-neutral-400">{HINT[t]}</span>
              </span>
            </button>
          );
        })}
      </div>
      {error && (
        <div className="mt-3">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </Dialog>
  );
}
