// Models → Installed: "Use models from another app". The user picks a ComfyUI, A1111 or Forge
// models folder once; Pinhole finds what it can run there and lists it with the installed
// models, without copying, moving or changing anything in that folder.
import { useCallback, useEffect, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { FolderSymlink, RefreshCw, TriangleAlert, X } from "lucide-react";
import { addLinkedFolder, asCoreError, listLinkedFolders, onModelsChanged, removeLinkedFolder, rescanLinkedFolders } from "../../lib/api";
import type { CoreError, LinkedFolder } from "../../lib/types";
import { Button, ErrorNotice, IconButton, Spinner } from "../../components/ui";
import { useTauriEvent } from "./lib/hooks";

/** "12 models · 40 style add-ons" (what Pinhole can use from the folder). */
export function linkedSummary(f: LinkedFolder): string {
  if (!f.available) return "Not connected. Connect its drive, then press Check again.";
  const parts = [
    `${f.models} ${f.models === 1 ? "model" : "models"}`,
    `${f.addons} style ${f.addons === 1 ? "add-on" : "add-ons"}`,
  ];
  if (f.parts > 0) parts.push(`${f.parts} ${f.parts === 1 ? "part" : "parts"} for other models`);
  return parts.join(" · ");
}

/** Button that asks for another app's models folder and adds it. */
export function AddLinkedFolderButton({ onAdded, onError }: { onAdded: (f: LinkedFolder) => void; onError: (e: CoreError) => void }) {
  const [busy, setBusy] = useState(false);
  const pick = async () => {
    try {
      const res = await openFileDialog({ directory: true, multiple: false, title: "Pick the models folder of ComfyUI, A1111 or Forge" });
      const folder = typeof res === "string" ? res : Array.isArray(res) ? (res[0] ?? null) : null;
      if (!folder) return;
      setBusy(true);
      onAdded(await addLinkedFolder(folder));
    } catch (e) {
      onError(asCoreError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Button variant="ghost" onClick={() => void pick()} disabled={busy} title="Use the models you already have in ComfyUI, A1111 or Forge, where they are">
      {busy ? <Spinner className="h-4 w-4" /> : <FolderSymlink className="h-4 w-4" />} Use models from another app
    </Button>
  );
}

/** The linked folders (shown only when there is one). `version` changes when one was added. */
export function LinkedFolders({ version }: { version: number }) {
  const [folders, setFolders] = useState<LinkedFolder[]>([]);
  const [error, setError] = useState<CoreError | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const refresh = useCallback(() => {
    listLinkedFolders()
      .then(setFolders)
      .catch((e) => setError(asCoreError(e)));
  }, []);
  useEffect(refresh, [refresh, version]);
  useTauriEvent(onModelsChanged, refresh);

  // Look again once per visit: new files the other app downloaded since then show up.
  const looked = useRef(false);
  useEffect(() => {
    if (looked.current || folders.length === 0) return;
    looked.current = true;
    rescanLinkedFolders().catch(() => undefined);
  }, [folders.length]);

  if (folders.length === 0) return error ? <ErrorNotice error={error} onDismiss={() => setError(null)} /> : null;

  const remove = async (f: LinkedFolder) => {
    setRemoving(f.id);
    setError(null);
    try {
      await removeLinkedFolder(f.id);
      refresh();
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setRemoving(null);
    }
  };
  const scanning = folders.some((f) => f.scanning);

  return (
    <section className="rounded-xl border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900" aria-label="Models from other apps">
      <div className="flex items-center justify-between gap-3 border-b border-neutral-100 px-4 py-2.5 dark:border-neutral-800">
        <div>
          <h3 className="text-sm font-semibold">Models from other apps</h3>
          <p className="text-xs text-neutral-500">Used where they are. Pinhole doesn't copy, change or delete anything in these folders.</p>
        </div>
        <Button size="sm" variant="ghost" onClick={() => void rescanLinkedFolders(true).catch((e) => setError(asCoreError(e)))} disabled={scanning}>
          <RefreshCw className="h-3.5 w-3.5" /> Check again
        </Button>
      </div>
      <ul className="divide-y divide-neutral-100 dark:divide-neutral-800">
        {folders.map((f) => (
          <li key={f.id} className="flex items-start justify-between gap-3 px-4 py-2.5 text-sm">
            <div className="min-w-0">
              <div className="font-medium text-neutral-900 dark:text-neutral-100">{f.name}</div>
              <div className="font-mono text-[11px] break-all text-neutral-500">{f.path}</div>
              <div className="mt-0.5 flex items-center gap-1.5 text-xs text-neutral-600 dark:text-neutral-400" aria-live="polite">
                {f.scanning ? (
                  <>
                    <Spinner className="h-3 w-3 text-amber-500" /> Looking through the folder… Big folders take a minute.
                  </>
                ) : !f.available ? (
                  <>
                    <TriangleAlert className="h-3.5 w-3.5 text-amber-600" /> {linkedSummary(f)}
                  </>
                ) : (
                  linkedSummary(f)
                )}
              </div>
            </div>
            <IconButton size="sm" label={`Stop using ${f.name}`} onClick={() => void remove(f)} disabled={removing === f.id}>
              {removing === f.id ? <Spinner className="h-4 w-4" /> : <X className="h-4 w-4" />}
            </IconButton>
          </li>
        ))}
      </ul>
      {error && (
        <div className="px-4 pb-3">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}
    </section>
  );
}
