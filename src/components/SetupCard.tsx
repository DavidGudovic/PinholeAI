// "Pinhole still needs" card (Create): everything missing before the first picture (image
// engine, safety check, and a model when none is installed) with one button that gets it
// all. Shown after Skip setup, or whenever one of them is missing. The single buttons
// elsewhere (top bar, Settings → Engine, the error fixes) keep working.
import { useCallback, useEffect, useMemo, useState } from "react";
import { CircleCheck, Download, PackageOpen } from "lucide-react";
import * as api from "../lib/api";
import type { CoreError, RecommendedPick, SafetyCheckStatus } from "../lib/types";
import { formatBytes } from "../lib/format";
import { isActiveDownload } from "../lib/state/model";
import { useAppState } from "../lib/state/store";
import { useActions } from "../lib/state/AppProvider";
import { tagGroup, useTaggedGroup } from "../tabs/models/lib/downloads";
import { useEngine, useOnHardwareChange, useTauriEvent } from "../tabs/models/lib/hooks";
import { Button, ErrorNotice, Spinner } from "./ui";

/** Create model offered when none is installed, in this order. */
const MODEL_ROLES = ["realistic", "anime"];

export interface SetupItem {
  key: "engine" | "check" | "model";
  label: string;
  bytes: number;
  downloading: boolean;
}

/** Button text: "Get all" for three, "Get both" for two, the item's own name for one. */
export function setupButtonLabel(items: SetupItem[]): string {
  const left = items.filter((i) => !i.downloading);
  const total = left.reduce((n, i) => n + i.bytes, 0);
  const size = total > 0 ? ` (${formatBytes(total)})` : "";
  if (left.length === 0) return "Downloading… (see Downloads)";
  if (left.length >= 3) return `Get all${size}`;
  if (left.length === 2) return `Get both${size}`;
  const one = left[0];
  const name = one.key === "engine" ? "the engine" : one.key === "check" ? "the safety check" : one.label.replace(/^A model: /, "");
  return `Get ${name}${size}`;
}

/** The model to offer: the first recommended Create pick that fits and can be downloaded. */
export function modelOffer(picks: RecommendedPick[] | null): RecommendedPick | null {
  for (const role of MODEL_ROLES) {
    const p = picks?.find((x) => x.role === role);
    if (p && p.title && !p.installed && !p.unavailableReason && p.fit === "fits") return p;
  }
  return null;
}

export function SetupCard({ needsModel, className = "" }: { needsModel: boolean; className?: string }) {
  const engine = useEngine();
  const actions = useActions();
  const downloads = useAppState((s) => s.downloads);
  const [check, setCheck] = useState<SafetyCheckStatus | null>(null);
  const [picks, setPicks] = useState<RecommendedPick[] | null>(null);
  // Asked for here and not answered yet (a licence question can be open): counts as downloading,
  // so a second click doesn't ask again.
  const [requested, setRequested] = useState<ReadonlySet<SetupItem["key"]>>(new Set());
  const [error, setError] = useState<CoreError | null>(null);

  const refreshCheck = useCallback(() => {
    api
      .safetyCheckStatus()
      .then(setCheck)
      .catch(() => undefined);
  }, []);
  const loadPicks = useCallback(() => {
    if (!needsModel) return;
    api
      .getRecommended()
      .then(setPicks)
      .catch(() => undefined);
  }, [needsModel]);
  useEffect(refreshCheck, [refreshCheck]);
  useEffect(loadPicks, [loadPicks]);
  useTauriEvent(api.onModelsChanged, loadPicks);
  useOnHardwareChange(loadPicks);

  // The check's download (started here, by the engine setup, or elsewhere) finishing changes its status.
  const checkActive = downloads.some((d) => d.kind === "safetyCheck" && isActiveDownload(d));
  useEffect(refreshCheck, [checkActive, refreshCheck]);
  const pick = needsModel ? modelOffer(picks) : null;
  const modelGroup = useTaggedGroup(`rec:${pick?.role ?? ""}`, (g) => g.kind === "model" && !!pick && g.label === pick.title);
  const modelActive = !!pick && !!modelGroup && isActiveDownload(modelGroup);

  const items = useMemo(() => {
    const out: SetupItem[] = [];
    const st = engine.status;
    if (st && !st.installed) out.push({ key: "engine", label: "Image engine", bytes: st.downloadBytes ?? 0, downloading: engine.busy || requested.has("engine") });
    if (check && !check.ready) out.push({ key: "check", label: "Safety check", bytes: check.downloadBytes, downloading: check.downloading || checkActive || requested.has("check") });
    if (pick) out.push({ key: "model", label: `A model: ${pick.title}`, bytes: pick.downloadBytes, downloading: modelActive || requested.has("model") });
    return out;
  }, [engine.status, engine.busy, check, checkActive, pick, modelActive, requested]);

  // Only a model missing is the Create tab's own "Get a model" page.
  if (!items.some((i) => i.key !== "model")) return null;

  const getAll = () => {
    setError(null);
    void actions.refreshDownloads().catch(() => undefined);
    const left = items.filter((i) => !i.downloading).map((i) => i.key);
    setRequested((r) => new Set([...r, ...left]));
    // Each request reports its own failure as soon as it happens.
    const run = (key: SetupItem["key"], job: () => Promise<unknown>, doneWhen: "started" | "finished") => {
      const p = job();
      const settle = () =>
        setRequested((r) => {
          const next = new Set(r);
          next.delete(key);
          return next;
        });
      p.catch((e) => {
        const ce = api.asCoreError(e);
        if (ce.code !== "cancelled") setError(ce);
      }).finally(() => {
        if (doneWhen === "finished") settle();
        refreshCheck();
      });
      if (doneWhen === "started") p.then(settle, settle);
    };
    // The engine setup also starts the safety check; asking for it again joins that download.
    // engine.install shows its own failure (engine.error) and keeps engine.busy while it runs.
    if (left.includes("engine")) run("engine", () => engine.install(), "started");
    if (left.includes("check")) run("check", () => api.installSafetyCheck(), "finished");
    if (left.includes("model") && pick) {
      const role = pick.role;
      // Resolves once the download is queued (after any licence question).
      run("model", () => api.installRecommended(role).then(({ groupId }) => tagGroup(`rec:${role}`, groupId)), "started");
    }
  };

  const allDownloading = items.every((i) => i.downloading);
  return (
    <section aria-label="Pinhole still needs" className={`${className} rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm text-amber-950 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-100`}>
      <div className="flex items-center gap-2 font-medium">
        <PackageOpen className="h-4 w-4 shrink-0" /> Pinhole still needs
      </div>
      <ul className="mt-2 space-y-1">
        {items.map((i) => (
          <li key={i.key} className="flex items-center justify-between gap-3">
            <span className="min-w-0 truncate">{i.label}</span>
            <span className="shrink-0 text-xs text-amber-800 tabular-nums dark:text-amber-300">
              {i.downloading ? (
                <span className="inline-flex items-center gap-1">
                  <Spinner className="h-3 w-3" /> Downloading
                </span>
              ) : i.bytes > 0 ? (
                formatBytes(i.bytes)
              ) : null}
            </span>
          </li>
        ))}
      </ul>
      <div className="mt-3 flex items-center gap-2">
        <Button size="sm" variant="primary" disabled={allDownloading} onClick={getAll}>
          {allDownloading ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />}
          {setupButtonLabel(items)}
        </Button>
        {allDownloading && (
          <span className="inline-flex items-center gap-1 text-xs text-amber-800 dark:text-amber-300">
            <CircleCheck className="h-3.5 w-3.5" /> You can keep going while it downloads.
          </span>
        )}
      </div>
      {(error ?? engine.error) && (
        <div className="mt-3">
          <ErrorNotice
            error={(error ?? engine.error)!}
            onDismiss={() => {
              setError(null);
              engine.setError(null);
            }}
          />
        </div>
      )}
    </section>
  );
}
