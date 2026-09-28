// Models → Installed: installed models + style add-ons, delete, "Add a file I already have".
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { CircleCheck, Compass, FilePlus2, Puzzle, Trash2, TriangleAlert } from "lucide-react";
import { addLocalModel, asCoreError, confirmFamily, deleteModel, listLoras, listModels, onModelsChanged, previewDelete } from "../../lib/api";
import type { AddFileResult, CoreError, DeletePreview, InstalledLora, InstalledModel } from "../../lib/types";
import { formatBytes } from "../../lib/format";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { Badge, Button, Dialog, ErrorNotice, IconButton, Spinner, VramBadge } from "../../components/ui";
import { EmptyState, FamilyPicker, Skeleton } from "./controls";
import { InstallDialog } from "./InstallDialog";
import { useTauriEvent } from "./lib/hooks";
import { baseName, isModelFile, lastUsedText } from "./lib/words";

type NeedsChoice = NonNullable<AddFileResult["needsChoice"]>;

export function InstalledView({ active, onBrowse }: { active: boolean; onBrowse: () => void }) {
  const [models, setModels] = useState<InstalledModel[] | null>(null);
  const [loras, setLoras] = useState<InstalledLora[] | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<{ id: string; name: string } | null>(null);
  const [missingFor, setMissingFor] = useState<InstalledModel | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [addError, setAddError] = useState<CoreError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [choice, setChoice] = useState<NeedsChoice | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [m, l] = await Promise.all([listModels(), listLoras()]);
      setModels(m);
      setLoras(l);
      setError(null);
    } catch (e) {
      setError(asCoreError(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  useTauriEvent(onModelsChanged, () => void refresh());

  const handleResult = useCallback(
    (r: AddFileResult) => {
      if (r.needsChoice) {
        setChoice(r.needsChoice);
        return;
      }
      if (r.model) setNotice(`Added “${r.model.friendlyName}”${r.model.familyLabel ? ` as ${r.model.familyLabel}` : ""}.`);
      else if (r.lora) setNotice(`Added “${r.lora.friendlyName}” as a style add-on.`);
      void refresh();
    },
    [refresh],
  );

  const addPath = useCallback(
    async (path: string) => {
      setAddError(null);
      setNotice(null);
      if (!isModelFile(path)) {
        setAddError({ code: "invalid", message: "Pinhole can only add .safetensors and .gguf files. Older .ckpt and .pt files can hide harmful code.", details: null });
        return;
      }
      setAdding(baseName(path));
      try {
        handleResult(await addLocalModel(path));
      } catch (e) {
        setAddError(asCoreError(e));
      } finally {
        setAdding(null);
      }
    },
    [handleResult],
  );

  const pickFile = async () => {
    try {
      const res = await openFileDialog({
        multiple: false,
        directory: false,
        title: "Add a model file",
        filters: [{ name: "Models", extensions: ["safetensors", "gguf"] }],
      });
      const path = typeof res === "string" ? res : Array.isArray(res) ? (res[0] ?? null) : null;
      if (path) await addPath(path);
    } catch (e) {
      setAddError(asCoreError(e));
    }
  };

  // Drag a .safetensors/.gguf onto the window while this view is on screen.
  useEffect(() => {
    if (!active) return;
    let off: (() => void) | null = null;
    let alive = true;
    try {
      getCurrentWebview()
        .onDragDropEvent((e) => {
          if (e.payload.type !== "drop") return;
          const path = e.payload.paths.find(isModelFile);
          if (path) void addPath(path);
        })
        .then((un) => (alive ? (off = un) : un()))
        .catch(() => undefined);
    } catch {
      /* not running inside Tauri (browser mock) */
    }
    return () => {
      alive = false;
      off?.();
    };
  }, [active, addPath]);

  const nothing = models !== null && loras !== null && models.length === 0 && loras.length === 0;
  const totalBytes = (models ?? []).reduce((a, m) => a + m.sizeBytes, 0) + (loras ?? []).reduce((a, l) => a + l.sizeBytes, 0);

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-sm text-neutral-600 dark:text-neutral-400">
          {!models || !loras
            ? "Loading…"
            : nothing
              ? "Nothing installed yet."
              : `${models.length} ${models.length === 1 ? "model" : "models"} · ${loras.length} style ${loras.length === 1 ? "add-on" : "add-ons"} · ${formatBytes(totalBytes)} on disk`}
        </p>
        <div className="flex gap-2">
          <Button onClick={() => void pickFile()} disabled={!!adding}>
            <FilePlus2 className="h-4 w-4" /> Add a file I already have
          </Button>
          <Button variant="ghost" onClick={onBrowse}>
            <Compass className="h-4 w-4" /> Browse CivitAI
          </Button>
        </div>
      </div>

      {adding && (
        <div className="flex items-center gap-3 rounded-xl border border-neutral-200 bg-white px-4 py-3 text-sm dark:border-neutral-800 dark:bg-neutral-900" aria-live="polite">
          <Spinner className="h-4 w-4 text-amber-500" />
          <span>
            Adding <b>{adding}</b> — Pinhole checks what kind of model it is and copies it into your Data folder. Big files take a minute.
          </span>
        </div>
      )}
      {addError && <ErrorNotice error={addError} onDismiss={() => setAddError(null)} />}
      {notice && (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-emerald-200 bg-emerald-50 px-3 py-2 text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-500/10 dark:text-emerald-200">
          <span className="inline-flex items-center gap-2">
            <CircleCheck className="h-4 w-4" /> {notice}
          </span>
          <button className="text-xs hover:underline" onClick={() => setNotice(null)}>
            Dismiss
          </button>
        </div>
      )}
      {error && <ErrorNotice error={error} />}

      {nothing && (
        <section className="space-y-3">
          <div>
            <h2 className="text-base font-semibold">Start with a recommended model</h2>
            <p className="text-sm text-neutral-600 dark:text-neutral-400">Nothing is installed yet. These are the best picks for your graphics card — one click each.</p>
          </div>
          <RecommendedCards />
        </section>
      )}

      {models === null && !error && (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-14 w-full" />
          ))}
        </div>
      )}

      {models && models.length > 0 && (
        <TableCard title="Models">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-neutral-500">
                <Th>Name</Th>
                <Th className="hidden lg:table-cell">Kind</Th>
                <Th>Size</Th>
                <Th>Graphics memory</Th>
                <Th className="hidden md:table-cell">Last used</Th>
                <Th className="w-10">
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
              {models.map((m) => (
                <tr key={m.id} className="align-top hover:bg-neutral-50 dark:hover:bg-neutral-800/40">
                  <Td>
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="font-medium text-neutral-900 dark:text-neutral-100">{m.friendlyName}</span>
                      {m.styleBadge && <Badge>{m.styleBadge}</Badge>}
                      {m.isEditModel && <Badge tone="blue">Edit</Badge>}
                    </div>
                    <div className="mt-0.5 text-xs text-neutral-500 lg:hidden">{m.familyLabel ?? "Unknown kind"}</div>
                    {m.licenseNote && <div className="mt-0.5 text-[11px] text-neutral-500">License: {m.licenseNote}</div>}
                    {m.missingComponents.length > 0 && (
                      <div className="mt-1.5 flex flex-wrap items-center gap-2 text-xs text-amber-800 dark:text-amber-300">
                        <span className="inline-flex items-start gap-1">
                          <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
                          Missing parts: {m.missingComponents.join(", ")}. It won't run until they're downloaded.
                        </span>
                        {m.civitaiVersionId != null && (
                          <Button size="sm" onClick={() => setMissingFor(m)}>
                            Get missing parts
                          </Button>
                        )}
                      </div>
                    )}
                  </Td>
                  <Td className="hidden text-neutral-600 lg:table-cell dark:text-neutral-400">{m.familyLabel ?? "Unknown"}</Td>
                  <Td className="whitespace-nowrap text-neutral-600 tabular-nums dark:text-neutral-400">{formatBytes(m.sizeBytes)}</Td>
                  <Td>{m.vram ? <VramBadge vram={m.vram} fit={m.fit} /> : <span className="text-xs text-neutral-400">Unknown</span>}</Td>
                  <Td className="hidden whitespace-nowrap text-neutral-600 md:table-cell dark:text-neutral-400">{lastUsedText(m.lastUsed)}</Td>
                  <Td>
                    <IconButton size="sm" label={`Delete ${m.friendlyName}`} onClick={() => setDeleteTarget({ id: m.id, name: m.friendlyName })}>
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableCard>
      )}

      {loras && loras.length > 0 && (
        <TableCard title="Style add-ons" icon={<Puzzle className="h-4 w-4 text-neutral-400" />}>
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-neutral-500">
                <Th>Name</Th>
                <Th>Works with</Th>
                <Th className="hidden md:table-cell">Trigger words</Th>
                <Th>Size</Th>
                <Th className="w-10">
                  <span className="sr-only">Actions</span>
                </Th>
              </tr>
            </thead>
            <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
              {loras.map((l) => (
                <tr key={l.id} className="align-top hover:bg-neutral-50 dark:hover:bg-neutral-800/40">
                  <Td className="font-medium text-neutral-900 dark:text-neutral-100">{l.friendlyName}</Td>
                  <Td className="text-neutral-600 dark:text-neutral-400">{l.baseModel ?? l.familyId ?? "Any"}</Td>
                  <Td className="hidden md:table-cell">
                    {l.trainedWords.length ? (
                      <div className="flex flex-wrap gap-1">
                        {l.trainedWords.map((w) => (
                          <Badge key={w} tone="amber">
                            {w}
                          </Badge>
                        ))}
                      </div>
                    ) : (
                      <span className="text-xs text-neutral-400">None needed</span>
                    )}
                  </Td>
                  <Td className="whitespace-nowrap text-neutral-600 tabular-nums dark:text-neutral-400">{formatBytes(l.sizeBytes)}</Td>
                  <Td>
                    <IconButton size="sm" label={`Delete ${l.friendlyName}`} onClick={() => setDeleteTarget({ id: l.id, name: l.friendlyName })}>
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </Td>
                </tr>
              ))}
            </tbody>
          </table>
        </TableCard>
      )}

      {nothing && (
        <EmptyState icon={<FilePlus2 className="h-6 w-6" />} title="Already have a model file?">
          Use <b>Add a file I already have</b> or drop a .safetensors or .gguf file here. Pinhole works out what kind of model it is.
        </EmptyState>
      )}

      <DeleteDialog target={deleteTarget} onClose={() => setDeleteTarget(null)} onDeleted={() => void refresh()} />
      <FamilyChoiceDialog
        choice={choice}
        onClose={() => setChoice(null)}
        onDone={(r) => {
          setChoice(null);
          handleResult(r);
        }}
      />
      <InstallDialog versionId={missingFor?.civitaiVersionId ?? null} title={missingFor?.friendlyName} onClose={() => setMissingFor(null)} />
    </div>
  );
}

function TableCard({ title, icon, children }: { title: string; icon?: ReactNode; children: ReactNode }) {
  return (
    <section className="overflow-hidden rounded-xl border border-neutral-200 bg-white shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
      <h2 className="flex items-center gap-2 border-b border-neutral-100 px-4 py-2.5 text-sm font-semibold dark:border-neutral-800">
        {icon}
        {title}
      </h2>
      <div className="overflow-x-auto">{children}</div>
    </section>
  );
}

function Th({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <th className={`px-4 py-2 font-medium ${className}`}>{children}</th>;
}

function Td({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <td className={`px-4 py-2.5 ${className}`}>{children}</td>;
}

function DeleteDialog({ target, onClose, onDeleted }: { target: { id: string; name: string } | null; onClose: () => void; onDeleted: () => void }) {
  const [preview, setPreview] = useState<DeletePreview | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!target) return;
    let alive = true;
    setPreview(null);
    setError(null);
    previewDelete(target.id)
      .then((p) => alive && setPreview(p))
      .catch((e) => alive && setError(asCoreError(e)));
    return () => {
      alive = false;
    };
  }, [target]);

  const confirm = async () => {
    if (!target) return;
    setBusy(true);
    setError(null);
    try {
      await deleteModel(target.id);
      onDeleted();
      onClose();
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setBusy(false);
    }
  };

  const total = preview?.files.reduce((a, f) => a + f.sizeBytes, 0) ?? 0;
  const orphans = preview?.files.filter((f) => f.reason === "orphanComponent").length ?? 0;

  return (
    <Dialog
      open={!!target}
      onClose={onClose}
      title={`Delete “${target?.name ?? ""}”?`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" onClick={() => void confirm()} disabled={!preview || busy}>
            {busy ? <Spinner className="h-3.5 w-3.5" /> : <Trash2 className="h-4 w-4" />} Delete{preview ? ` · frees ${formatBytes(total)}` : ""}
          </Button>
        </>
      }
    >
      {!preview && !error && (
        <div className="flex items-center gap-2 text-sm text-neutral-500">
          <Spinner className="h-4 w-4" /> Checking which files can go…
        </div>
      )}
      {preview && (
        <div className="space-y-3 text-sm">
          <p className="text-neutral-700 dark:text-neutral-300">
            These files will be removed from your Data folder{orphans > 0 ? ", including parts no other model uses" : ""}. You can download the model again later.
          </p>
          <ul className="divide-y divide-neutral-100 rounded-lg border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
            {preview.files.map((f) => (
              <li key={f.relPath} className="flex items-center gap-3 px-3 py-2">
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-xs text-neutral-800 dark:text-neutral-200" title={f.relPath}>
                    {f.relPath}
                  </span>
                  <span className="text-[11px] text-neutral-500">{f.reason === "model" ? "Model file" : "Shared part no other model uses"}</span>
                </span>
                <span className="shrink-0 text-xs text-neutral-600 tabular-nums dark:text-neutral-400">{formatBytes(f.sizeBytes)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
      {error && (
        <div className="mt-3">
          <ErrorNotice error={error} />
        </div>
      )}
    </Dialog>
  );
}

function FamilyChoiceDialog({ choice, onClose, onDone }: { choice: NeedsChoice | null; onClose: () => void; onDone: (r: AddFileResult) => void }) {
  const [family, setFamily] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  useEffect(() => {
    setFamily(null);
    setError(null);
  }, [choice]);

  const confirm = async () => {
    if (!choice || !family) return;
    setBusy(true);
    setError(null);
    try {
      onDone(await confirmFamily(choice.token, family));
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open={!!choice}
      onClose={onClose}
      title="Which kind of model is this?"
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={() => void confirm()} disabled={!family || busy}>
            {busy && <Spinner className="h-3.5 w-3.5" />} Add model
          </Button>
        </>
      }
    >
      {choice && (
        <div className="space-y-3 text-sm">
          <p className="text-neutral-700 dark:text-neutral-300">
            <span className="font-mono text-xs">{choice.fileName}</span> could be one of these. They look the same inside the file, so Pinhole needs your help. Pick the one
            the download page mentions.
          </p>
          <FamilyPicker candidates={choice.candidates} value={family} onChange={setFamily} />
          {error && <ErrorNotice error={error} />}
        </div>
      )}
    </Dialog>
  );
}
