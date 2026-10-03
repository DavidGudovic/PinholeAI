// Settings → Saved pictures → the folder Save puts pictures in (Pictures/Pinhole by
// default). Changing it doesn't move pictures that were already saved.
import { useCallback, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderInput, FolderOpen, RotateCcw } from "lucide-react";
import { asCoreError, openOutputsFolder, saveFolderInfo, setSaveFolder } from "../lib/api";
import type { CoreError, SaveFolderInfo } from "../lib/types";
import { Button, ErrorNotice, Spinner } from "../components/ui";
import { Skeleton } from "../tabs/models/controls";

export function SaveFolderSection() {
  const [info, setInfo] = useState<SaveFolderInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  const refresh = useCallback(() => {
    saveFolderInfo()
      .then(setInfo)
      .catch((e) => setError(asCoreError(e)));
  }, []);
  useEffect(refresh, [refresh]);

  const use = async (folder: string | null) => {
    setError(null);
    setBusy(true);
    try {
      setInfo(await setSaveFolder(folder));
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setBusy(false);
    }
  };

  const pick = async () => {
    try {
      const res = await openFileDialog({ directory: true, multiple: false, title: "Pick a folder for saved pictures", defaultPath: info?.path });
      const folder = typeof res === "string" ? res : Array.isArray(res) ? (res[0] ?? null) : null;
      if (folder) await use(folder);
    } catch (e) {
      setError(asCoreError(e));
    }
  };

  return (
    <div className="space-y-1.5">
      <div className="text-sm font-medium text-neutral-800 dark:text-neutral-200">Saved pictures folder</div>
      {info ? (
        <p className="rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 font-mono text-xs break-all text-neutral-700 dark:border-neutral-800 dark:bg-neutral-950 dark:text-neutral-300">
          {info.path}
        </p>
      ) : (
        <Skeleton className="h-9 w-full" />
      )}
      <div className="flex flex-wrap gap-1.5">
        <Button size="sm" onClick={() => void pick()} disabled={busy || !info}>
          {busy ? <Spinner className="h-3.5 w-3.5" /> : <FolderInput className="h-4 w-4" />} Change…
        </Button>
        {info?.custom && (
          <Button size="sm" variant="ghost" onClick={() => void use(null)} disabled={busy}>
            <RotateCcw className="h-4 w-4" /> Use default
          </Button>
        )}
        <Button size="sm" variant="ghost" onClick={() => void openOutputsFolder().catch((e) => setError(asCoreError(e)))}>
          <FolderOpen className="h-4 w-4" /> Open folder
        </Button>
      </div>
      <p className="text-xs text-neutral-500">Save puts pictures here. Changing it doesn't move pictures you already saved.</p>
      {info?.earlierPath && <p className="text-xs break-all text-neutral-500">Pictures you saved before are still in {info.earlierPath}.</p>}
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
    </div>
  );
}
