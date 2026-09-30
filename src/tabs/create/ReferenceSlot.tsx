// Optional reference picture for Create ("in the style of this picture", "the same character").
// Drop, paste, choose a file or pick a picture from this session. Models that can't use one (only
// FLUX.2 and Qwen-Image 2.1 can) still show the button, then offer a model that can, or Edit.
// Like every session image it stays in memory; it never goes into a preset.
import { useState } from "react";
import { ImagePlus, Trash, TriangleAlert } from "lucide-react";
import { DropTarget, useFilePicker, useImagePaste } from "../../components/ImageDrop";
import { Button, ErrorNotice, IconButton, Spinner } from "../../components/ui";
import * as api from "../../lib/api";
import { useActions } from "../../lib/state/AppProvider";
import { referenceModel, takesReference } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import { useAppState, useDispatch } from "../../lib/state/store";
import type { CoreError, InstalledModel } from "../../lib/types";

/** How many of this session's pictures the empty slot offers. */
const RECENT = 6;

export function ReferenceSlot({ model }: { model: InstalledModel | null }) {
  const refId = useAppState((s) => s.create.refImageId);
  const ref = useAppState((s) => (refId ? s.images[refId] : undefined));
  const tab = useAppState((s) => s.tab);
  const models = useAppState((s) => s.models);
  const results = useAppState((s) => s.results);
  const images = useAppState((s) => s.images);
  const dispatch = useDispatch();
  const actions = useActions();
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  const able = takesReference(model);
  // Paste only where a picture is expected, so a model that can't use one keeps paste for text.
  const pasting = able || !!ref;
  const load = async (f: File) => {
    setError(null);
    setImporting(true);
    try {
      await actions.importCreateReference(f);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setImporting(false);
    }
  };
  useImagePaste(tab === "create" && pasting, (f) => void load(f));
  const picker = useFilePicker((f) => void load(f));

  if (ref) {
    const suggest = able ? null : referenceModel(models);
    return (
      <div className="space-y-2 rounded-xl border border-neutral-200 p-2 dark:border-neutral-800">
        <DropTarget onFile={(f) => void load(f)} label="Drop to replace the reference picture">
          <div className="flex items-center gap-3">
            <img src={ref.url} alt="Reference picture" className="h-12 w-12 shrink-0 rounded-md object-cover" draggable={false} />
            <p className="min-w-0 flex-1 text-xs text-neutral-500">
              <span className="font-medium text-neutral-700 dark:text-neutral-300">Reference picture.</span> Say how to use it, like “in the style of the picture” or “the
              same dog on a beach”.
            </p>
            <IconButton label="Remove the reference picture" size="sm" onClick={() => dispatch({ type: "createSetRef", ref: null })}>
              <Trash className="h-3.5 w-3.5" />
            </IconButton>
          </div>
        </DropTarget>
        {!able && (
          <div className="flex flex-wrap items-center gap-2 text-xs text-amber-800 dark:text-amber-300">
            <span className="inline-flex items-start gap-1.5">
              <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
              {model?.friendlyName ?? "This model"} can't use a reference picture.
              {!suggest && " Edit can keep the same character in a new scene."}
            </span>
            {suggest ? (
              <Button size="sm" onClick={() => dispatch({ type: "selectModel", modelId: suggest.id })}>
                Switch to {suggest.friendlyName}
              </Button>
            ) : (
              <Button size="sm" onClick={() => actions.sameCharacter(ref.id)}>
                Use it in Edit
              </Button>
            )}
          </div>
        )}
        {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      </div>
    );
  }

  const recent = able ? results.filter((r) => images[r.id]).slice(0, RECENT) : [];
  return (
    <DropTarget onFile={(f) => void load(f)} label="Drop to use as the reference picture">
      {picker.input}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="ghost" disabled={importing} onClick={picker.open} title={
            able
              ? `Optional: make something in the style of a picture, or with the same character or subject. Drop, paste (${modKey}+V) or choose one.`
              : "Optional: make something in the style of a picture, or with the same character or subject. Drop or choose one."
          }>
          {importing ? <Spinner className="h-3.5 w-3.5" /> : <ImagePlus className="h-3.5 w-3.5" />} Add a reference picture
        </Button>
        {recent.length > 0 && (
          <div role="group" aria-label="Use a picture from this session" className="flex gap-1">
            {recent.map((r) => (
              <button
                key={r.id}
                type="button"
                title="Use as the reference picture"
                aria-label="Use as the reference picture"
                className="h-7 w-7 overflow-hidden rounded-md opacity-80 ring-amber-500 hover:opacity-100 hover:ring-2 focus-visible:ring-2 focus-visible:outline-none"
                onClick={() => dispatch({ type: "createSetRef", ref: images[r.id] })}
              >
                <img src={images[r.id].url} alt="" className="h-full w-full object-cover" draggable={false} />
              </button>
            ))}
          </div>
        )}
      </div>
      {error && (
        <div className="mt-2">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </DropTarget>
  );
}
