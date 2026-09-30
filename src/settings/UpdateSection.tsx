// Settings → Updates. Pinhole never checks on its own (SPEC §4 rule 6): the only
// request to GitHub happens when the user presses "Check for updates".
import { useState } from "react";
import { CircleCheck, Download, ExternalLink, RefreshCw } from "lucide-react";
import { asCoreError, checkForUpdates, installUpdate, openReleasePage } from "../lib/api";
import type { CoreError, UpdateCheck } from "../lib/types";
import { formatBytes } from "../lib/format";
import { Button, ErrorNotice, Spinner } from "../components/ui";
import { GroupProgress } from "../tabs/models/controls";
import { cancelGroup, useTaggedGroup } from "../tabs/models/lib/downloads";

type Phase = "idle" | "checking" | "checked" | "installing" | "restarting";

const RESTART = " Save your pictures first: Pinhole closes and reopens, and unsaved pictures are lost.";
const HOW: Record<string, string> = {
  installer: "Pinhole downloads the new installer, checks it and runs it." + RESTART,
  portable: "Pinhole downloads the new version, checks it and swaps it in. Your Data folder is not touched." + RESTART,
  appImage: "Pinhole downloads the new AppImage, checks it and replaces this one." + RESTART,
  manual: "This copy can't update itself. Download the new version from the release page.",
};

export function UpdateSection({ offline }: { offline: boolean }) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [result, setResult] = useState<UpdateCheck | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const group = useTaggedGroup("app-update", (g) => g.kind === "appUpdate");
  // Settings was closed and reopened while an update downloads: keep showing it.
  const running = phase === "installing" || (phase === "idle" && group != null);

  const check = async () => {
    setError(null);
    setPhase("checking");
    try {
      setResult(await checkForUpdates());
      setPhase("checked");
    } catch (e) {
      setError(asCoreError(e));
      setPhase("idle");
    }
  };

  const install = async (version: string) => {
    setError(null);
    setPhase("installing");
    try {
      await installUpdate(version);
      // On success the app quits and the new version starts.
      setPhase("restarting");
    } catch (e) {
      const err = asCoreError(e);
      if (err.code !== "cancelled") setError(err);
      // update_restart: the new files are already in place, only a restart is missing.
      if (err.code === "update_restart") setResult((r) => (r ? { ...r, update: null } : r));
      setPhase(err.code === "update_restart" ? "idle" : "checked");
    }
  };

  const openPage = (version: string | null) => void openReleasePage(version).catch((e) => setError(asCoreError(e)));
  const update = result?.update ?? null;
  const busy = phase === "checking" || running || phase === "restarting";

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0 text-sm">
          {phase === "checked" && result && !update ? (
            <span className="inline-flex items-center gap-1.5 font-medium text-emerald-700 dark:text-emerald-400">
              <CircleCheck className="h-4 w-4" /> You have the newest version ({result.currentVersion})
            </span>
          ) : update ? (
            <span className="font-medium text-neutral-800 dark:text-neutral-200">Pinhole {update.version} is available</span>
          ) : (
            <span className="text-neutral-700 dark:text-neutral-300">Pinhole never checks by itself.</span>
          )}
          <p className="text-xs text-neutral-500">
            {update
              ? HOW[update.installMode]
              : offline
                ? "Turn off Offline mode to check."
                : "Only when you press this, Pinhole asks GitHub for its newest release. Your prompts, pictures and settings are not sent."}
          </p>
        </div>
        {!update && !running && (
          <Button size="sm" disabled={busy || offline} onClick={() => void check()}>
            {phase === "checking" ? <Spinner className="h-3.5 w-3.5" /> : <RefreshCw className="h-3.5 w-3.5" />}
            Check for updates
          </Button>
        )}
      </div>

      {update && !running && phase !== "restarting" && (
        <div className="flex flex-wrap gap-1.5">
          {update.installMode !== "manual" ? (
            <Button size="sm" variant="primary" disabled={offline} onClick={() => void install(update.version)}>
              <Download className="h-3.5 w-3.5" /> Update and restart{update.sizeBytes ? ` (${formatBytes(update.sizeBytes)})` : ""}
            </Button>
          ) : (
            <Button size="sm" variant="primary" onClick={() => openPage(update.version)}>
              <ExternalLink className="h-3.5 w-3.5" /> Open download page
            </Button>
          )}
          {update.installMode !== "manual" && (
            <Button size="sm" variant="ghost" onClick={() => openPage(update.version)}>
              <ExternalLink className="h-3.5 w-3.5" /> What's new
            </Button>
          )}
        </div>
      )}

      {running &&
        (group ? (
          <GroupProgress group={group} onCancel={() => void cancelGroup(group.groupId)} />
        ) : (
          <span className="flex items-center gap-2 text-xs text-neutral-500">
            <Spinner className="h-3 w-3" /> Getting the update ready…
          </span>
        ))}
      {phase === "restarting" && (
        <span className="flex items-center gap-2 text-xs text-neutral-500" aria-live="polite">
          <Spinner className="h-3 w-3" /> Restarting Pinhole…
        </span>
      )}
      {error?.code === "updates_unavailable" ? (
        <div className="space-y-2 rounded-lg bg-neutral-100 p-3 text-xs text-neutral-600 dark:bg-neutral-800/60 dark:text-neutral-400" role="note">
          <p>{error.message}</p>
          <Button size="sm" variant="ghost" onClick={() => openPage(null)}>
            <ExternalLink className="h-3.5 w-3.5" /> Open release page
          </Button>
        </div>
      ) : (
        error && <ErrorNotice error={error} onDismiss={() => setError(null)} />
      )}
    </div>
  );
}
