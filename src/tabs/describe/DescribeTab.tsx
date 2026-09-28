// Describe (img2text), SPEC §5.3.
import { useCallback, useEffect, useRef, useState } from "react";
import { Copy, Download, ImagePlus, ScanText, Sparkles, Tags, TextQuote } from "lucide-react";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { DropTarget, DropZone, useFilePicker, useImagePaste } from "../../components/ImageDrop";
import { AutoTextarea, Button, Kbd, Segmented, Spinner } from "../../components/ui";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { GroupProgress } from "../models/controls";
import * as api from "../../lib/api";
import { formatBytes } from "../../lib/format";
import type { CaptionerStatus, CoreError, DescribeStyle } from "../../lib/types";
import { useActions, usePrimaryAction } from "../../lib/state/AppProvider";
import { useElapsed } from "../../lib/state/hooks";
import { isActiveDownload } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";

export function DescribeTab() {
  const tab = useAppState((s) => s.tab);
  const d = useAppState((s) => s.describe);
  const img = useAppState((s) => (s.describe.imageId ? s.images[s.describe.imageId] : undefined));
  const downloads = useAppState((s) => s.downloads);
  const dispatch = useDispatch();
  const store = useStore();
  const actions = useActions();

  const [status, setStatus] = useState<CaptionerStatus | null>(null);
  const [statusError, setStatusError] = useState(false);
  const [busy, setBusy] = useState<{ at: number } | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [importing, setImporting] = useState(false);
  const [installGroup, setInstallGroup] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const elapsed = useElapsed(busy?.at ?? null, !!busy);
  const runId = useRef(0);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.captionerStatus());
      setStatusError(false);
    } catch {
      setStatusError(true);
    }
  }, []);
  useEffect(() => {
    if (tab === "describe") void refreshStatus();
  }, [tab, refreshStatus]);
  // Models changed (e.g. the edit model's encoder arrived) → captioner may be available now.
  const models = useAppState((s) => s.models);
  useEffect(() => {
    void refreshStatus();
  }, [models, refreshStatus]);
  // The describer download started here, or anywhere else (Recommended cards, First run):
  // its group kind is "captioner".
  const dl =
    (installGroup ? downloads.find((x) => x.groupId === installGroup) : undefined) ??
    [...downloads].reverse().find((x) => x.kind === "captioner" && isActiveDownload(x));
  useEffect(() => {
    if (dl?.state === "done") void refreshStatus();
  }, [dl?.state, refreshStatus]);

  const load = async (f: File) => {
    setError(null);
    setImporting(true);
    try {
      await actions.importToDescribe(f);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setImporting(false);
    }
  };
  useImagePaste(tab === "describe", (f) => void load(f));
  const picker = useFilePicker((f) => void load(f));

  const describe = async () => {
    const s = store.getState().describe;
    if (!s.imageId || busy) return;
    const id = ++runId.current;
    setError(null);
    setBusy({ at: Date.now() });
    try {
      const text = await api.describeImage(s.imageId, s.style);
      if (id === runId.current) dispatch({ type: "patchDescribe", patch: { text: text.trim() } });
    } catch (e) {
      if (id === runId.current) setError(api.asCoreError(e));
    } finally {
      if (id === runId.current) setBusy(null);
    }
  };
  usePrimaryAction("describe", () => void describe());

  const install = async () => {
    setInstalling(true);
    setError(null);
    try {
      const started = await api.installCaptioner();
      setInstallGroup(started.groupId);
      void actions.refreshDownloads().catch(() => undefined);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setInstalling(false);
    }
  };

  const cancelInstall = async (groupId: string) => {
    try {
      await api.cancelDownload(groupId);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      void actions.refreshDownloads().catch(() => undefined);
    }
  };

  const unavailable = status && !status.available;
  const installingNow = !!dl && isActiveDownload(dl);

  return (
    <div className="grid h-full grid-cols-[minmax(0,1fr)_minmax(360px,440px)]">
      <DropTarget onFile={(f) => void load(f)} className="flex min-h-0 min-w-0 flex-col bg-neutral-100 dark:bg-neutral-950" label="Drop to describe this image">
        {picker.input}
        {img ? (
          <>
            <div className="flex min-h-0 flex-1 items-center justify-center p-6">
              <img src={img.url} alt="Image to describe" className="max-h-full max-w-full rounded-lg object-contain shadow-lg ring-1 ring-black/5 dark:ring-white/10" draggable={false} />
            </div>
            <div className="flex shrink-0 items-center justify-center gap-2 pb-4">
              <Button size="sm" variant="ghost" onClick={picker.open}>
                <ImagePlus className="h-3.5 w-3.5" /> Another image
              </Button>
              <span className="text-xs text-neutral-500 tabular-nums">
                {img.width}×{img.height}
              </span>
            </div>
          </>
        ) : (
          <div className="flex min-h-0 flex-1 p-6">
            <DropZone onFile={(f) => void load(f)} title="Add an image to describe" busy={importing}>
              {importing && <Spinner className="mt-3 h-4 w-4" />}
            </DropZone>
          </div>
        )}
      </DropTarget>

      <aside aria-label="Description" className="flex min-h-0 flex-col border-l border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pt-4 pb-5">
          <div>
            <h1 className="text-base font-semibold">Describe an image</h1>
            <p className="mt-0.5 text-sm text-neutral-500">Turn a picture into a prompt you can reuse. Runs on this computer.</p>
          </div>

          <div>
            <div className="mb-1.5 text-sm text-neutral-600 dark:text-neutral-400">Write it as</div>
            <Segmented
              stretch
              ariaLabel="Description style"
              value={d.style}
              onChange={(v) => dispatch({ type: "patchDescribe", patch: { style: v } })}
              options={[
                { value: "sentence" as DescribeStyle, label: (<><TextQuote className="h-3.5 w-3.5" /> Sentence</>), title: "Natural description — good for Flux, Z-Image, Qwen" },
                { value: "tags" as DescribeStyle, label: (<><Tags className="h-3.5 w-3.5" /> Tags</>), title: "Comma-separated tags — good for anime SDXL models" },
              ]}
            />
            <p className="mt-1.5 text-xs text-neutral-500">
              {d.style === "sentence" ? "A natural description. Best for Flux, Z-Image and Qwen models." : "Comma-separated tags. Best for anime SDXL models (Pony, Illustrious)."}
            </p>
          </div>

          {unavailable && (
            <div className="space-y-3 rounded-xl border border-neutral-200 p-3 dark:border-neutral-800">
              <div className="text-sm font-medium">Describing needs a small vision model</div>
              <p className="text-xs text-neutral-500">
                {status.downloadBytes > 0 ? `A one-time ${formatBytes(status.downloadBytes)} download. ` : ""}If you install the Qwen Image Edit model, Describe reuses it for free.
              </p>
              {installingNow && dl ? (
                <GroupProgress group={dl} compact onCancel={() => void cancelInstall(dl.groupId)} />
              ) : (
                <Button variant="primary" onClick={() => void install()} disabled={installing}>
                  {installing ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-4 w-4" />}
                  Get the describer{status.downloadBytes ? ` (${formatBytes(status.downloadBytes)})` : ""}
                </Button>
              )}
              <details className="text-xs text-neutral-500">
                <summary className="cursor-pointer select-none">Other options</summary>
                <div className="mt-2">
                  <RecommendedCards roles={["describe"]} compact />
                </div>
              </details>
            </div>
          )}
          {statusError && !status && <p className="text-xs text-neutral-500">Couldn’t check the describer. You can still try.</p>}

          <div>
            <div className="mb-1.5 flex items-center justify-between">
              <label htmlFor="describe-out" className="text-sm font-medium">
                Description
              </label>
              {d.text && <span className="text-xs text-neutral-400">You can edit it</span>}
            </div>
            {busy ? (
              <div className="space-y-2 rounded-lg border border-neutral-200 p-3 dark:border-neutral-800" role="status">
                <div className="flex items-center gap-2 text-sm text-neutral-600 dark:text-neutral-300">
                  <Spinner className="h-3.5 w-3.5 text-amber-500" /> Looking at the image… {elapsed > 2 ? `${elapsed} s` : ""}
                </div>
                <div className="pinhole-shimmer h-3 w-full rounded bg-neutral-100 dark:bg-neutral-800" />
                <div className="pinhole-shimmer h-3 w-4/5 rounded bg-neutral-100 dark:bg-neutral-800" />
                <div className="pinhole-shimmer h-3 w-3/5 rounded bg-neutral-100 dark:bg-neutral-800" />
                {elapsed > 6 && <p className="text-xs text-neutral-500">The first description takes longer while the describer starts.</p>}
              </div>
            ) : (
              <AutoTextarea
                id="describe-out"
                minRows={6}
                maxRows={16}
                value={d.text}
                placeholder={img ? "Press Describe to get a description." : "Add an image first."}
                onChange={(e) => dispatch({ type: "patchDescribe", patch: { text: e.target.value } })}
              />
            )}
          </div>

          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" disabled={!d.text.trim()} onClick={() => actions.useAsPrompt(d.text.trim())}>
              <Sparkles className="h-4 w-4" /> Use as prompt
            </Button>
            <Button variant="secondary" disabled={!d.text.trim()} onClick={() => void actions.copyTextToClipboard(d.text.trim(), "Description copied").catch((e) => setError(api.asCoreError(e)))}>
              <Copy className="h-4 w-4" /> Copy
            </Button>
          </div>
        </div>

        <div className="shrink-0 space-y-2 border-t border-neutral-200 px-5 py-4 dark:border-neutral-800">
          <Button variant="primary" size="lg" className="w-full" disabled={!img || !!busy || !!unavailable} onClick={() => void describe()}>
            <ScanText className="h-4 w-4" /> Describe
            <span className="ml-1 inline-flex gap-0.5 opacity-70">
              <Kbd>{modKey}</Kbd>
              <Kbd>Enter</Kbd>
            </span>
          </Button>
          {error && <ErrorWithFix error={error} onDismiss={() => setError(null)} onRetry={() => void describe()} />}
        </div>
      </aside>
    </div>
  );
}
