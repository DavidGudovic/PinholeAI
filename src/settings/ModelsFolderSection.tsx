// Settings → Models folder. Keep models in a folder you pick, e.g. on a drive that
// Windows and Linux share, so Pinhole on both uses the same files. Changing it moves
// the installed models there, then Pinhole restarts.
import { useCallback, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderInput, FolderOpen, RotateCcw, TriangleAlert } from "lucide-react";
import { asCoreError, changeModelsFolder, modelsFolderInfo, onModelsMove, openModelsFolder, previewModelsFolder } from "../lib/api";
import type { CoreError, ModelsFolderInfo, ModelsFolderPreview, ModelsMoveProgress } from "../lib/types";
import { formatBytes } from "../lib/format";
import { Badge, Button, Dialog, ErrorNotice, ProgressBar, Spinner } from "../components/ui";
import { Skeleton } from "../tabs/models/controls";
import { useTauriEvent } from "../tabs/models/lib/hooks";

/** Plain words for a Models folder that can't be used right now (Rust says the same). */
export const FOLDER_PROBLEM: Record<"missing" | "readOnly", string> = {
  missing: "Your Models folder isn't available. Connect or mount the drive it's on and restart Pinhole, or pick another folder in Settings.",
  readOnly:
    "Your Models folder is read-only right now, so Pinhole can't add models. If you dual boot with Windows, turn off Fast Startup in Windows and shut it down fully.",
};

type Phase = "idle" | "previewing" | "confirm" | "moving" | "restarting";

export function ModelsFolderSection() {
  const [info, setInfo] = useState<ModelsFolderInfo | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [target, setTarget] = useState<string | null>(null);
  const [preview, setPreview] = useState<ModelsFolderPreview | null>(null);
  const [progress, setProgress] = useState<ModelsMoveProgress | null>(null);
  const [error, setError] = useState<CoreError | null>(null);

  const refresh = useCallback(() => {
    modelsFolderInfo()
      .then(setInfo)
      .catch((e) => setError(asCoreError(e)));
  }, []);
  useEffect(refresh, [refresh]);
  useTauriEvent(onModelsMove, setProgress);

  const ask = async (folder: string | null) => {
    setError(null);
    setPhase("previewing");
    try {
      setPreview(await previewModelsFolder(folder));
      setTarget(folder);
      setPhase("confirm");
    } catch (e) {
      setError(asCoreError(e));
      setPhase("idle");
    }
  };

  const pick = async () => {
    try {
      const res = await openFileDialog({ directory: true, multiple: false, title: "Pick a folder for your models" });
      const folder = typeof res === "string" ? res : Array.isArray(res) ? (res[0] ?? null) : null;
      if (folder) await ask(folder);
    } catch (e) {
      setError(asCoreError(e));
    }
  };

  const move = async () => {
    setError(null);
    setProgress(null);
    setPhase("moving");
    try {
      await changeModelsFolder(target);
      // On success Pinhole restarts with the new folder.
      setPhase("restarting");
      refresh();
    } catch (e) {
      setError(asCoreError(e));
      setPhase("idle");
    }
  };

  const busy = phase === "previewing" || phase === "moving" || phase === "restarting";
  const dialogOpen = phase === "confirm" || phase === "moving" || phase === "restarting";

  return (
    <div className="space-y-3">
      {info ? (
        <>
          <div className="flex items-center gap-2">
            <Badge tone={info.custom ? "amber" : "neutral"}>{info.custom ? "Your folder" : "Inside the Data folder"}</Badge>
            <span className="text-xs text-neutral-500">
              {info.custom ? "Another Pinhole (for example on your other operating system) can use the same folder." : "Pick a shared drive to use one set of models from Windows and Linux."}
            </span>
          </div>
          <p className="rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 font-mono text-xs break-all text-neutral-700 dark:border-neutral-800 dark:bg-neutral-950 dark:text-neutral-300">
            {info.path}
          </p>
          {info.problem && (
            <p className="flex items-start gap-1.5 text-xs text-amber-800 dark:text-amber-300" role="note">
              <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
              <span>{FOLDER_PROBLEM[info.problem]}</span>
            </p>
          )}
        </>
      ) : (
        <Skeleton className="h-9 w-full" />
      )}
      <div className="flex flex-wrap gap-1.5">
        <Button size="sm" onClick={() => void pick()} disabled={busy}>
          {phase === "previewing" ? <Spinner className="h-3.5 w-3.5" /> : <FolderInput className="h-4 w-4" />} Change…
        </Button>
        {info?.custom && (
          <Button size="sm" variant="ghost" onClick={() => void ask(null)} disabled={busy}>
            <RotateCcw className="h-4 w-4" /> Use the Data folder again
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => void openModelsFolder().catch((e) => setError(asCoreError(e)))}>
          <FolderOpen className="h-4 w-4" /> Open
        </Button>
      </div>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}

      <Dialog
        open={dialogOpen}
        onClose={() => phase === "confirm" && setPhase("idle")}
        title={
          phase !== "confirm" ? "Moving your models…" : preview?.isDefault ? "Move your models back to the Data folder?" : "Move your models to this folder?"
        }
        footer={
          phase === "confirm" ? (
            <>
              <Button variant="ghost" onClick={() => setPhase("idle")}>
                Cancel
              </Button>
              <Button variant="primary" onClick={() => void move()}>
                Move and restart
              </Button>
            </>
          ) : undefined
        }
      >
        {preview && (
          <div className="space-y-3 text-sm text-neutral-700 dark:text-neutral-300">
            <p className="rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 font-mono text-xs break-all dark:border-neutral-800 dark:bg-neutral-950">{preview.path}</p>
            {phase === "confirm" && (
              <>
                <p>
                  {preview.files === 0
                    ? "You have no models installed yet, so nothing needs to move."
                    : `Pinhole moves your installed models there (${formatBytes(preview.bytes)}).${
                        preview.sameDrive ? " It's on the same drive, so this is quick." : " It's on another drive, so Pinhole copies and checks each file first. Big models take a few minutes."
                      }`}
                </p>
                {preview.existingModels > 0 && (
                  <p>
                    This folder already has {preview.existingModels} {preview.existingModels === 1 ? "model" : "models"} from another Pinhole. You'll see them too, and
                    models you both have aren't copied twice.
                  </p>
                )}
                {info?.custom && <p>If Pinhole on your other operating system uses the current folder, it won't find these models there any more.</p>}
                <p className="text-xs text-neutral-500">
                  Deleting a model in a shared folder removes it for every Pinhole that uses the folder. Save your pictures first: Pinhole restarts when the move is
                  done, and unsaved pictures are lost.
                </p>
              </>
            )}
            {phase === "moving" && (
              <div className="space-y-2" aria-live="polite">
                <ProgressBar value={progress?.doneBytes ?? 0} max={progress?.totalBytes || 1} indeterminate={!progress || progress.totalBytes === 0} />
                <p className="text-xs text-neutral-500">
                  {progress && progress.totalBytes > 0
                    ? `${formatBytes(progress.doneBytes)} of ${formatBytes(progress.totalBytes)}${progress.fileName ? ` · ${progress.fileName}` : ""}`
                    : "Moving your models…"}
                </p>
                <p className="text-xs text-neutral-500">Keep Pinhole open. If something goes wrong, every file is put back where it was.</p>
              </div>
            )}
            {phase === "restarting" && (
              <span className="flex items-center gap-2 text-xs text-neutral-500" aria-live="polite">
                <Spinner className="h-3 w-3" /> Restarting Pinhole…
              </span>
            )}
          </div>
        )}
      </Dialog>
    </div>
  );
}
