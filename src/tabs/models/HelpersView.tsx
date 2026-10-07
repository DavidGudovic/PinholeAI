// Models → Helpers: the small language models behind Describe and "Improve my prompt".
// They come from Pinhole's own list (not CivitAI), run on this computer, and work offline once downloaded.
import { useState } from "react";
import { CircleCheck, Download, Trash2 } from "lucide-react";
import { Badge, Button, Dialog, ErrorNotice, Spinner } from "../../components/ui";
import * as api from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { useHelperModels } from "../../lib/helpers";
import { useActions } from "../../lib/state/AppProvider";
import { isActiveDownload } from "../../lib/state/model";
import { useAppState } from "../../lib/state/store";
import type { CoreError, Fit, HelperModel } from "../../lib/types";

export const FIT_WORDS: Record<Fit, { label: string; tone: "green" | "amber" | "red"; title: string }> = {
  fits: { label: "Fits", tone: "green", title: "Runs on your graphics card." },
  tight: { label: "Tight", tone: "amber", title: "Part of it runs on the processor, so it is slower." },
  tooBig: { label: "Too big", tone: "red", title: "More than this computer's memory. Pick the smaller one." },
};

export function HelpersView() {
  const models = useHelperModels();
  const actions = useActions();
  const busyDownload = useAppState((s) => s.downloads.some((d) => d.kind === "captioner" && isActiveDownload(d)));
  const [starting, setStarting] = useState<string | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [removing, setRemoving] = useState<HelperModel | null>(null);

  const get = async (m: HelperModel) => {
    setStarting(m.id);
    setError(null);
    try {
      await api.installCaptioner(m.id);
      void actions.refreshDownloads().catch(() => undefined);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setStarting(null);
    }
  };

  return (
    <div className="space-y-4">
      <p className="text-sm text-neutral-600 dark:text-neutral-400">
        Helpers are small language models that run on this computer. They write the description in Describe and the fuller prompt in Improve.
        Pick which one each uses next to its button or in Settings; Automatic uses the larger one when it is installed.
      </p>
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
      {!models ? (
        <div className="flex justify-center py-10">
          <Spinner className="h-5 w-5 text-neutral-400" />
        </div>
      ) : (
        <ul className="grid gap-3 md:grid-cols-2">
          {models.map((m) => (
            <li key={m.id} className="flex flex-col gap-2 rounded-xl border border-neutral-200 p-4 dark:border-neutral-800">
              <div className="flex items-start justify-between gap-2">
                <h2 className="text-sm font-semibold">{m.title}</h2>
                {m.fit && (
                  <span title={FIT_WORDS[m.fit].title}>
                    <Badge tone={FIT_WORDS[m.fit].tone}>{FIT_WORDS[m.fit].label}</Badge>
                  </span>
                )}
              </div>
              <p className="text-xs text-neutral-500">{m.note}</p>
              <p className="text-xs text-neutral-500 tabular-nums">{m.installed ? `${formatBytes(m.sizeBytes)} on this computer` : `${formatBytes(m.downloadBytes || m.sizeBytes)} download`}</p>
              <div className="mt-auto flex flex-wrap items-center gap-2 pt-1">
                {m.installed ? (
                  <>
                    <span className="inline-flex items-center gap-1.5 text-xs font-medium text-emerald-700 dark:text-emerald-400">
                      <CircleCheck className="h-3.5 w-3.5" /> Installed
                    </span>
                    {m.removable ? (
                      <Button size="sm" variant="ghost" onClick={() => setRemoving(m)}>
                        <Trash2 className="h-3.5 w-3.5" /> Delete
                      </Button>
                    ) : (
                      <span className="text-xs text-neutral-500">Came with another model</span>
                    )}
                  </>
                ) : (
                  <Button size="sm" variant="primary" disabled={busyDownload || starting === m.id} onClick={() => void get(m)}>
                    {starting === m.id ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />} Get
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      <RemoveDialog target={removing} onClose={() => setRemoving(null)} />
    </div>
  );
}

function RemoveDialog({ target, onClose }: { target: HelperModel | null; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const confirm = async () => {
    if (!target) return;
    setBusy(true);
    setError(null);
    try {
      await api.deleteHelper(target.id);
      onClose();
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={!!target}
      onClose={onClose}
      title={`Delete “${target?.title ?? ""}”?`}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} data-autofocus>
            Cancel
          </Button>
          <Button variant="danger" onClick={() => void confirm()} disabled={busy}>
            {busy ? <Spinner className="h-3.5 w-3.5" /> : <Trash2 className="h-4 w-4" />} Delete{target ? ` · frees ${formatBytes(target.sizeBytes)}` : ""}
          </Button>
        </>
      }
    >
      <p className="text-sm text-neutral-700 dark:text-neutral-300">Describe and Improve go back to Automatic. Pinhole offers to download it again when you need it.</p>
      {error && (
        <div className="mt-3">
          <ErrorNotice error={error} />
        </div>
      )}
    </Dialog>
  );
}
