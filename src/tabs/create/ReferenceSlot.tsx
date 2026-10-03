// Optional reference picture for Create ("in the style of this picture", "the same character").
// Shown only for models that take one (FLUX.2, Qwen-Image 2.1), or while a picture is set; the
// "Same character" result action finds the right model. Drop, paste, choose a file or pick a
// picture from this session. Like every session image it stays in memory; it never goes into a preset.
import { useState } from "react";
import { ImagePlus, Trash, TriangleAlert } from "lucide-react";
import { DropTarget, useFilePicker } from "../../components/ImageDrop";
import { SessionStrip, useSessionPictures } from "../../components/SessionPictures";
import { Button, ErrorNotice, IconButton, Spinner } from "../../components/ui";
import * as api from "../../lib/api";
import { useActions } from "../../lib/state/AppProvider";
import { referenceModel, takesReference } from "../../lib/state/model";
import { PhotoNotice } from "../../components/PhotoNotice";
import { modKey } from "../../lib/state/platform";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import type { CoreError, InstalledModel } from "../../lib/types";

/** How many of this session's pictures the empty slot offers. */
const RECENT = 6;

export function ReferenceSlot({ model }: { model: InstalledModel | null }) {
  const refId = useAppState((s) => s.create.refImageId);
  const ref = useAppState((s) => (refId ? s.images[refId] : undefined));
  const models = useAppState((s) => s.models);
  const recent = useSessionPictures([], RECENT);
  const dispatch = useDispatch();
  const store = useStore();
  const actions = useActions();
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  const able = takesReference(model);
  const shown = able || !!ref;
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
  const picker = useFilePicker((f) => void load(f));
  if (!shown) return null;

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
        <PhotoNotice imageId={ref.id} verb="use" />
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

  return (
    <DropTarget onFile={(f) => void load(f)} label="Drop to use as the reference picture">
      {picker.input}
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" variant="ghost" disabled={importing} onClick={picker.open} title={`Optional: make something in the style of a picture, or with the same character or subject. Drop, paste (${modKey}+V) or choose one.`}>
          {importing ? <Spinner className="h-3.5 w-3.5" /> : <ImagePlus className="h-3.5 w-3.5" />} Add a reference picture
        </Button>
        <SessionStrip pictures={recent} title="Use as the reference picture" onPick={(p) => {
            const now = store.getState().images[p.id];
            if (now) dispatch({ type: "createSetRef", ref: now });
          }} />
      </div>
      {error && (
        <div className="mt-2">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </DropTarget>
  );
}
