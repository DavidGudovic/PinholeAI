import { useState, type ReactNode } from "react";
import { Download, Layers, RotateCcw, ScanText, Settings, Sparkles, WandSparkles, X } from "lucide-react";
import * as api from "../lib/api";
import { formatBytes } from "../lib/format";
import type { GroupStatus } from "../lib/types";
import { useActions } from "../lib/state/AppProvider";
import { isActiveDownload, type TabId } from "../lib/state/model";
import { useAppState } from "../lib/state/store";
import { Logo } from "./Logo";
import { ErrorWithFix } from "./ErrorWithFix";
import { Button, IconButton, Popover, ProgressBar, Spinner, cx, focusRing } from "./ui";

const TABS: { id: TabId; label: string; icon: ReactNode }[] = [
  { id: "create", label: "Create", icon: <Sparkles className="h-4 w-4" /> },
  { id: "edit", label: "Edit", icon: <WandSparkles className="h-4 w-4" /> },
  { id: "describe", label: "Describe", icon: <ScanText className="h-4 w-4" /> },
  { id: "models", label: "Models", icon: <Layers className="h-4 w-4" /> },
];

export function TopBar({ onOpenSettings }: { onOpenSettings: () => void }) {
  const tab = useAppState((s) => s.tab);
  const jobKind = useAppState((s) => s.job?.kind ?? null);
  const actions = useActions();
  const busyTab: TabId | null = jobKind === "edit" ? "edit" : jobKind ? "create" : null;

  return (
    <header className="flex h-13 shrink-0 items-center gap-3 border-b border-neutral-200 bg-white px-3 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex items-center gap-2 pr-1 pl-1">
        <Logo className="h-7 w-7 rounded-[7px] dark:ring-1 dark:ring-white/15" />
        <span className="hidden text-[15px] font-semibold tracking-tight md:inline">Pinhole</span>
      </div>

      <nav role="tablist" aria-label="Sections" className="flex items-center gap-0.5">
        {TABS.map((t) => {
          const active = t.id === tab;
          return (
            <button
              key={t.id}
              type="button"
              role="tab"
              aria-selected={active}
              aria-controls={`tab-${t.id}`}
              onClick={() => actions.setTab(t.id)}
              className={cx(
                "relative inline-flex h-9 items-center gap-1.5 rounded-lg px-3 text-sm font-medium transition-colors",
                focusRing,
                active
                  ? "bg-neutral-100 text-neutral-950 dark:bg-neutral-800 dark:text-white"
                  : "text-neutral-500 hover:bg-neutral-100/70 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800/60 dark:hover:text-white",
              )}
            >
              <span className={cx(active ? "text-amber-600 dark:text-amber-400" : "")}>{t.icon}</span>
              {t.label}
              {busyTab === t.id && !active && <Spinner className="h-3 w-3 text-amber-500" />}
            </button>
          );
        })}
      </nav>

      <div className="ml-auto flex items-center gap-1.5">
        <EngineChip />
        <DownloadsButton />
        <Button
          variant="ghost"
          size="md"
          onClick={() => void actions.clearSession()}
          title="Start over: clears the prompt fields and every unsaved image"
          className="px-2.5"
        >
          <RotateCcw className="h-4 w-4" />
          <span className="hidden lg:inline">Reset</span>
          <span className="sr-only lg:hidden">Reset</span>
        </Button>
        <IconButton label="Settings" onClick={onOpenSettings}>
          <Settings className="h-[18px] w-[18px]" />
        </IconButton>
      </div>
    </header>
  );
}

// ------------------------------------------------------------------ engine status

function EngineChip() {
  const engine = useAppState((s) => s.engine);
  const models = useAppState((s) => s.models);
  const jobPhase = useAppState((s) => s.job?.progress?.phase ?? null);
  const jobModel = useAppState((s) => s.job?.progress?.modelLabel ?? null);
  const [installing, setInstalling] = useState(false);
  const actions = useActions();
  if (!engine) return null;
  const modelName = (id: string | null) => (models ?? []).find((m) => m.id === id)?.friendlyName ?? null;

  const chip = "inline-flex h-8 items-center gap-2 rounded-full px-3 text-xs font-medium";
  if (engine.loading || jobPhase === "loadingModel") {
    const name = jobModel ?? modelName(engine.loadedModelId);
    return (
      <span className={cx(chip, "bg-amber-50 text-amber-900 dark:bg-amber-500/10 dark:text-amber-200")} role="status">
        <Spinner className="h-3 w-3" />
        <span className="hidden max-w-56 truncate sm:inline">Loading {name ?? "model"}…</span>
      </span>
    );
  }
  if (engine.installing) {
    return (
      <span className={cx(chip, "bg-neutral-100 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300")} role="status">
        <Spinner className="h-3 w-3" /> Setting up the engine…
      </span>
    );
  }
  if (!engine.installed) {
    return (
      <Button
        size="sm"
        variant="secondary"
        disabled={installing}
        onClick={async () => {
          setInstalling(true);
          try {
            actions.onEngine(await api.installEngine());
          } catch (e) {
            actions.toast(api.asCoreError(e).message, { tone: "error" });
          } finally {
            setInstalling(false);
          }
        }}
      >
        {installing ? <Spinner className="h-3 w-3" /> : <Download className="h-3.5 w-3.5" />}
        Get the engine
      </Button>
    );
  }
  if (engine.error) {
    return (
      <Popover
        align="end"
        width={360}
        trigger={(p) => (
          <button {...p} type="button" className={cx(chip, focusRing, "bg-red-50 text-red-800 hover:bg-red-100 dark:bg-red-500/10 dark:text-red-300")}>
            <span className="h-2 w-2 rounded-full bg-red-500" /> Engine problem
          </button>
        )}
      >
        <div className="p-2">
          {/* The real failure (e.g. "This model couldn't be loaded…"), engine output behind Details, and a fix where there is one. */}
          <ErrorWithFix error={{ code: engine.errorCode ?? "engine_failed", message: engine.error, details: engine.errorDetails ?? null }} />
        </div>
      </Popover>
    );
  }
  const loaded = modelName(engine.loadedModelId);
  if (!loaded) return null;
  return (
    <span className={cx(chip, "hidden text-neutral-500 md:inline-flex dark:text-neutral-400")} title="Loaded and ready">
      <span className="h-2 w-2 rounded-full bg-emerald-500" />
      <span className="max-w-44 truncate">{loaded}</span>
    </span>
  );
}

// ------------------------------------------------------------------ downloads

function DownloadsButton() {
  const downloads = useAppState((s) => s.downloads);
  const actions = useActions();
  if (!downloads.length) return null;
  const active = downloads.filter(isActiveDownload);
  const done = active.reduce((a, d) => a + d.downloadedBytes, 0);
  const total = active.reduce((a, d) => a + d.totalBytes, 0);
  const pct = total ? done / total : 0;
  const failed = downloads.some((d) => d.state === "failed");

  return (
    <Popover
      align="end"
      width={380}
      trigger={(p) => (
        <button
          {...p}
          type="button"
          aria-label={active.length ? `${active.length} download${active.length > 1 ? "s" : ""} in progress` : "Downloads"}
          title="Downloads"
          className={cx(
            "relative inline-flex h-9 items-center gap-1.5 rounded-lg px-2.5 text-sm text-neutral-600 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800",
            focusRing,
          )}
        >
          {active.length ? <ProgressRing value={pct} /> : <Download className="h-4 w-4" />}
          {active.length > 0 && <span className="tabular-nums text-xs font-medium">{Math.round(pct * 100)}%</span>}
          {failed && !active.length && <span className="absolute top-1.5 right-1.5 h-2 w-2 rounded-full bg-red-500" />}
        </button>
      )}
    >
      <div className="w-full p-1.5">
        <div className="flex items-center justify-between px-1.5 pt-1 pb-2">
          <span className="text-sm font-semibold">Downloads</span>
          {downloads.length > active.length && (
            <button type="button" className={cx("rounded text-xs text-neutral-500 hover:text-neutral-900 dark:hover:text-white", focusRing)} onClick={() => actions.clearFinishedDownloads()}>
              Clear finished
            </button>
          )}
        </div>
        <ul className="space-y-1">
          {[...downloads].reverse().map((d) => (
            <DownloadRow key={d.groupId} d={d} />
          ))}
        </ul>
      </div>
    </Popover>
  );
}

function DownloadRow({ d }: { d: GroupStatus }) {
  const active = isActiveDownload(d);
  const [cancelling, setCancelling] = useState(false);
  const state =
    d.state === "queued"
      ? "Waiting…"
      : d.state === "verifying"
        ? "Checking the file…"
        : d.state === "done"
          ? "Done"
          : d.state === "failed"
            ? "Failed"
            : d.state === "cancelled"
              ? "Cancelled"
              : `${formatBytes(d.downloadedBytes)} of ${formatBytes(d.totalBytes)}`;
  return (
    <li className="rounded-lg px-2 py-2 hover:bg-neutral-50 dark:hover:bg-neutral-800/50">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-medium">{d.label}</div>
          <div className={cx("mt-0.5 text-xs", d.state === "failed" ? "text-red-600 dark:text-red-400" : "text-neutral-500")}>
            {state}
            {d.fileCount > 1 && active && ` · file ${Math.min(d.fileIndex + 1, d.fileCount)} of ${d.fileCount}`}
          </div>
          {d.error && <div className="mt-0.5 text-xs text-red-600 dark:text-red-400">{d.error}</div>}
        </div>
        {active && (
          <IconButton
            label={`Cancel ${d.label}`}
            size="sm"
            disabled={cancelling}
            onClick={async () => {
              setCancelling(true);
              await api.cancelDownload(d.groupId).catch(() => undefined);
              setCancelling(false);
            }}
          >
            <X className="h-3.5 w-3.5" />
          </IconButton>
        )}
      </div>
      {active && <ProgressBar className="mt-2" value={d.downloadedBytes} max={d.totalBytes || 1} indeterminate={d.state !== "downloading" || !d.totalBytes} />}
    </li>
  );
}

function ProgressRing({ value }: { value: number }) {
  const r = 7;
  const c = 2 * Math.PI * r;
  return (
    <svg viewBox="0 0 18 18" className="h-[18px] w-[18px] -rotate-90" aria-hidden>
      <circle cx="9" cy="9" r={r} fill="none" strokeWidth="2.5" className="stroke-neutral-200 dark:stroke-neutral-700" />
      <circle cx="9" cy="9" r={r} fill="none" strokeWidth="2.5" strokeLinecap="round" className="stroke-amber-500 transition-[stroke-dashoffset]" strokeDasharray={c} strokeDashoffset={c * (1 - Math.min(1, Math.max(0.03, value)))} />
    </svg>
  );
}
