// "Paste from CivitAI" dialog + the summary of what was applied.
// PRIVACY: pasted text lives in component state only; never logged or stored.
import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ClipboardPaste, Download, Info, LoaderCircle, TriangleAlert, X } from "lucide-react";
import { AutoTextarea, Badge, Button, Dialog, ErrorNotice, IconButton, ProgressBar, cx } from "../../components/ui";
import * as api from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { mapSampler, samplerLabel, schedulerLabel } from "../../lib/paste/map";
import { looksLikeGenerationData, parseGenerationData } from "../../lib/paste/parse";
import type { CoreError, ResolvedResource } from "../../lib/types";
import { useActions } from "../../lib/state/AppProvider";
import { readClipboardText } from "../../lib/state/platform";
import { isActiveDownload } from "../../lib/state/model";
import { useAppState, useStore } from "../../lib/state/store";
import { applyParsed, resourceKey, type PasteOutcome } from "./pasteApply";

export function PasteDialog({
  open,
  onClose,
  onApply,
  initialText,
}: {
  open: boolean;
  onClose: () => void;
  onApply: (text: string) => Promise<void>;
  initialText?: string;
}) {
  return open ? <PasteDialogInner onClose={onClose} onApply={onApply} initialText={initialText} /> : null;
}

function PasteDialogInner({ onClose, onApply, initialText }: { onClose: () => void; onApply: (text: string) => Promise<void>; initialText?: string }) {
  const [text, setText] = useState(initialText ?? "");
  // The text as last rendered, for the clipboard read below.
  const textRef = useRef(text);
  textRef.current = text;
  const [fromClipboard, setFromClipboard] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  // Try the clipboard once when the dialog opens (only fills in generation data).
  useEffect(() => {
    if (initialText) return;
    let alive = true;
    void readClipboardText().then((t) => {
      // Only into an empty box: text typed while the clipboard was read stays.
      if (alive && t && looksLikeGenerationData(t) && !textRef.current.trim()) {
        setText(t);
        setFromClipboard(true);
      }
    });
    return () => {
      alive = false;
    };
  }, [initialText]);

  const parsed = useMemo(() => (text.trim() ? parseGenerationData(text) : null), [text]);
  const found = useMemo(() => {
    if (!parsed) return [];
    const out: string[] = [];
    if (parsed.prompt) out.push("Prompt");
    if (parsed.negative) out.push("Negative prompt");
    if (parsed.steps != null) out.push(`${parsed.steps} steps`);
    if (parsed.sampler || parsed.scheduler) {
      const m = mapSampler(parsed.sampler, parsed.scheduler);
      const name = [m.sampler && samplerLabel(m.sampler), m.scheduler && schedulerLabel(m.scheduler)].filter(Boolean).join(" · ");
      if (name) out.push(name);
    }
    if (parsed.cfg != null) out.push(`CFG ${parsed.cfg}`);
    if (parsed.guidance != null) out.push(`Guidance ${parsed.guidance}`);
    if (parsed.seed != null) out.push(`Seed ${parsed.seed}`);
    if (parsed.width && parsed.height) out.push(`${parsed.width}×${parsed.height}`);
    if (parsed.clipSkip != null) out.push(`Clip skip ${parsed.clipSkip}`);
    if (parsed.hires) out.push("Hires fix");
    const ck = parsed.resources.filter((r) => r.type === "checkpoint").length;
    const lo = parsed.resources.filter((r) => r.type === "lora").length;
    if (ck) out.push("Model");
    if (lo) out.push(`${lo} LoRA${lo > 1 ? "s" : ""}`);
    return out;
  }, [parsed]);

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      await onApply(text);
      onClose();
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      wide
      title="Paste from CivitAI"
      description={
        <>
          On a CivitAI image page, click <span className="font-medium text-neutral-700 dark:text-neutral-300">Copy generation data</span>, then paste it here. Nothing you paste is saved.
        </>
      }
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!parsed || busy} onClick={() => void apply()}>
            {busy ? <LoaderCircle className="h-4 w-4 animate-spin" /> : <ClipboardPaste className="h-4 w-4" />}
            {parsed?.hasSettings ? "Apply settings" : "Use as prompt"}
          </Button>
        </>
      }
    >
      <div className="space-y-3">
        <AutoTextarea
          aria-label="Generation data"
          data-autofocus
          minRows={7}
          maxRows={14}
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setFromClipboard(false);
          }}
          className="font-mono text-xs"
          placeholder={"a lighthouse on a cliff at sunset, dramatic clouds\nNegative prompt: blurry, lowres\nSteps: 30, Sampler: DPM++ 2M Karras, CFG scale: 7, Seed: 1234, Size: 832x1216, …"}
        />
        {fromClipboard && (
          <p className="flex items-center gap-1.5 text-xs text-neutral-500">
            <Info className="h-3.5 w-3.5" /> Filled in from your clipboard.
          </p>
        )}
        {parsed && (
          <div className="rounded-lg bg-neutral-50 p-3 dark:bg-neutral-800/50">
            <div className="mb-2 text-xs font-medium text-neutral-500">{parsed.hasSettings ? "Found" : "No settings found — this will be used as the prompt"}</div>
            <div className="flex flex-wrap gap-1.5">
              {found.map((f) => (
                <Badge key={f}>{f}</Badge>
              ))}
            </div>
          </div>
        )}
        {error && <ErrorNotice error={error} />}
      </div>
    </Dialog>
  );
}

// ------------------------------------------------------------------ summary

export function PasteSummary({ outcome, onDismiss, onOutcome }: { outcome: PasteOutcome; onDismiss: () => void; onOutcome: (o: PasteOutcome) => void }) {
  const downloads = useAppState((s) => s.downloads);
  const store = useStore();
  const actions = useActions();
  const [groups, setGroups] = useState<Record<string, string>>({});
  const [starting, setStarting] = useState<Record<string, boolean>>({});
  const [errors, setErrors] = useState<Record<string, CoreError>>({});
  const [reapplying, setReapplying] = useState(false);
  const { resolved } = outcome;

  const get = async (r: ResolvedResource) => {
    if (r.installableVersionId == null) return;
    const k = resourceKey(r);
    setStarting((s) => ({ ...s, [k]: true }));
    setErrors((x) => {
      const n = { ...x };
      delete n[k];
      return n;
    });
    try {
      const started = await api.installCivitai(r.installableVersionId, null);
      setGroups((g) => ({ ...g, [k]: started.groupId }));
      void actions.refreshDownloads().catch(() => undefined);
    } catch (e) {
      setErrors((x) => ({ ...x, [k]: api.asCoreError(e) }));
    } finally {
      setStarting((s) => ({ ...s, [k]: false }));
    }
  };

  /** After a download finished: resolve again and re-apply the pasted settings (prompt untouched). */
  const reapply = async () => {
    setReapplying(true);
    try {
      await actions.refreshModels().catch(() => undefined);
      const res = await api.resolveCivitaiResources(outcome.parsed.resources);
      onOutcome(await applyParsed(outcome.parsed, res, null, store, actions, { setPrompt: false }));
    } catch (e) {
      actions.toast(api.asCoreError(e).message, { tone: "error" });
    } finally {
      setReapplying(false);
    }
  };

  const rows: { r: ResolvedResource; kind: "Model" | "LoRA" }[] = [];
  if (resolved?.checkpoint) rows.push({ r: resolved.checkpoint, kind: "Model" });
  for (const r of resolved?.loras ?? []) rows.push({ r, kind: "LoRA" });

  const row = ({ r, kind }: (typeof rows)[number]) => {
    const k = resourceKey(r);
    const gid = groups[k];
    const dl = gid ? downloads.find((d) => d.groupId === gid) : undefined;
    const weight = kind === "LoRA" && r.resource.weight != null ? ` at ${r.resource.weight}` : "";
    const using =
      r.installedId &&
      (kind === "Model" ? outcome.checkpointSelected : outcome.lorasAdded.includes(r.displayName));
    const incompatible = kind === "LoRA" && outcome.lorasIncompatible.includes(r.displayName);
    return (
      <li key={k} className="flex flex-wrap items-center gap-x-2 gap-y-1 py-1.5 text-sm">
        <span className="w-10 shrink-0 text-[11px] font-medium text-neutral-400 uppercase">{kind}</span>
        <span className="min-w-0 flex-1 truncate font-medium" title={r.displayName}>
          {r.displayName}
          {weight && <span className="font-normal text-neutral-500">{weight}</span>}
        </span>
        {using ? (
          <span className="inline-flex items-center gap-1 text-xs text-emerald-700 dark:text-emerald-400">
            <Check className="h-3.5 w-3.5" /> Using it
          </span>
        ) : incompatible ? (
          <span className="text-xs text-amber-700 dark:text-amber-400">Doesn’t fit this model</span>
        ) : r.installedId ? (
          <span className="text-xs text-neutral-500">Installed</span>
        ) : dl ? (
          isActiveDownload(dl) ? (
            <span className="flex w-40 items-center gap-2 text-xs text-neutral-500">
              <ProgressBar value={dl.downloadedBytes} max={dl.totalBytes || 1} indeterminate={!dl.totalBytes} />
              {dl.totalBytes ? `${Math.round((dl.downloadedBytes / dl.totalBytes) * 100)}%` : ""}
            </span>
          ) : dl.state === "done" ? (
            <Button size="sm" variant="primary" onClick={() => void reapply()} disabled={reapplying}>
              {reapplying ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Check className="h-3.5 w-3.5" />} Use it
            </Button>
          ) : (
            <span className="text-xs text-red-600">{dl.state === "cancelled" ? "Cancelled" : (dl.error ?? "Download failed")}</span>
          )
        ) : r.installableVersionId != null ? (
          <span className="flex items-center gap-2">
            {r.fit && <Badge tone={r.fit === "fits" ? "green" : r.fit === "tight" ? "amber" : "red"}>{r.fit === "fits" ? "Fits" : r.fit === "tight" ? "Tight" : "Too big"}</Badge>}
            <Button size="sm" onClick={() => void get(r)} disabled={starting[k]}>
              {starting[k] ? <LoaderCircle className="h-3.5 w-3.5 animate-spin" /> : <Download className="h-3.5 w-3.5" />}
              Get{r.downloadBytes ? ` (${formatBytes(r.downloadBytes)})` : ""}
            </Button>
          </span>
        ) : (
          <span className="text-xs text-neutral-500">{r.problem ?? "Not available"}</span>
        )}
        {errors[k] && (
          <div className="w-full">
            <ErrorNotice error={errors[k]} />
          </div>
        )}
      </li>
    );
  };

  const hasSkipped = outcome.skipped.length > 0 || outcome.ignoredKeys.length > 0 || (resolved?.ignored.length ?? 0) > 0;

  return (
    <div className="pinhole-pop rounded-xl border border-emerald-200 bg-emerald-50/60 p-3 text-sm dark:border-emerald-900/60 dark:bg-emerald-950/20" role="status">
      <div className="flex items-start gap-2">
        <Check className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600" />
        <div className="min-w-0 flex-1">
          <div className="font-medium">
            Settings applied{outcome.modelName ? ` for ${outcome.modelName}` : ""}
          </div>
          {outcome.applied.length ? (
            <ul className="mt-1.5 flex flex-wrap gap-1" aria-label="Applied">
              {outcome.applied.map((a) => (
                <li key={a} className="rounded-md bg-white/80 px-1.5 py-0.5 text-[11px] leading-4 text-neutral-700 ring-1 ring-emerald-200 ring-inset dark:bg-neutral-900/60 dark:text-neutral-300 dark:ring-emerald-900/60">
                  {a}
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-0.5 text-xs text-neutral-500">Nothing to apply</p>
          )}
        </div>
        <IconButton label="Dismiss summary" size="sm" onClick={onDismiss} className="-mt-1 -mr-1">
          <X className="h-4 w-4" />
        </IconButton>
      </div>

      {rows.length > 0 && <ul className="mt-2 divide-y divide-emerald-200/70 border-t border-emerald-200/70 dark:divide-emerald-900/40 dark:border-emerald-900/40">{rows.map(row)}</ul>}

      {outcome.resolveError && (
        <p className="mt-2 flex items-start gap-1.5 text-xs text-amber-800 dark:text-amber-300">
          <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
          Couldn’t look up the models it used: {outcome.resolveError.message}
        </p>
      )}

      {hasSkipped && (
        <details className="mt-2 text-xs text-neutral-600 dark:text-neutral-400">
          <summary className={cx("cursor-pointer select-none font-medium")}>Not applied ({outcome.skipped.length + (resolved?.ignored.length ?? 0) + (outcome.ignoredKeys.length ? 1 : 0)})</summary>
          <ul className="mt-1.5 space-y-1 pl-1">
            {outcome.skipped.map((s, i) => (
              <li key={i}>
                <span className="font-medium text-neutral-700 dark:text-neutral-300">{s.what}</span> — {s.why}
              </li>
            ))}
            {resolved?.ignored.map((r, i) => (
              <li key={`ig${i}`}>
                <span className="font-medium text-neutral-700 dark:text-neutral-300">{r.displayName}</span> — {r.problem ?? "not used by Pinhole"}
              </li>
            ))}
            {outcome.ignoredKeys.length > 0 && <li>Also not used: {outcome.ignoredKeys.join(", ")}</li>}
          </ul>
        </details>
      )}
    </div>
  );
}
