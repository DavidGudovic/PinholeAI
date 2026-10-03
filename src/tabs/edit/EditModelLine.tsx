// One slim line at the top of Edit while it opens in Restyle because no edit model is installed:
// the edit model changes one thing and keeps the rest. Gone once one is installed, or closed for
// this app session.
import { useCallback, useEffect, useState } from "react";
import { Download, WandSparkles, X } from "lucide-react";
import { asCoreError, getRecommended, installRecommended, onModelsChanged } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import type { CoreError, RecommendedPick } from "../../lib/types";
import { Button, ErrorNotice, Spinner, cx, focusRing } from "../../components/ui";
import { GroupProgress } from "../models/controls";
import { cancelGroup, tagGroup, useTaggedGroup } from "../models/lib/downloads";
import { isActive } from "../models/lib/words";
import { useTauriEvent } from "../models/lib/hooks";

let closedThisSession = false;
/** Tests only. */
export function resetEditModelLine() {
  closedThisSession = false;
}

export function EditModelLine() {
  const [pick, setPick] = useState<RecommendedPick | null>(null);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const [closed, setClosed] = useState(closedThisSession);
  // Same tag as the Edit card in RecommendedCards, so both show the same download.
  const group = useTaggedGroup("rec:edit", (g) => !!pick?.title && g.label === pick.title);

  const load = useCallback(() => {
    getRecommended()
      .then((picks) => setPick(picks.find((p) => p.role === "edit") ?? null))
      .catch(() => setPick(null));
  }, []);
  useEffect(load, [load]);
  useTauriEvent(onModelsChanged, load);

  const get = async () => {
    setError(null);
    setStarting(true);
    try {
      const { groupId } = await installRecommended("edit");
      tagGroup("rec:edit", groupId);
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setStarting(false);
    }
  };

  // Only an edit model that fits this graphics card and isn't installed yet.
  if (closed || !pick || pick.installed || !pick.title || pick.unavailableReason || pick.fit !== "fits") return null;
  const downloading = !!group && isActive(group);
  const size = pick.downloadBytes > 0 ? ` (${formatBytes(pick.downloadBytes)})` : "";
  return (
    <div className="space-y-2 rounded-lg border border-amber-200 bg-amber-50/70 px-3 py-2 dark:border-amber-900/60 dark:bg-amber-500/5" role="note">
      <div className="flex items-center gap-2 text-xs text-neutral-700 dark:text-neutral-300">
        <WandSparkles className="h-3.5 w-3.5 shrink-0 text-amber-600 dark:text-amber-400" aria-hidden />
        <span className="min-w-0 flex-1">To change one thing and keep the rest, get the edit model{size}.</span>
        {!downloading && (
          <Button variant="primary" size="sm" onClick={() => void get()} disabled={starting} aria-label={`Get ${pick.title}`}>
            {starting ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />}
            Get
          </Button>
        )}
        <button
          type="button"
          aria-label="Close"
          onClick={() => {
            closedThisSession = true;
            setClosed(true);
          }}
          className={cx("shrink-0 rounded p-0.5 text-neutral-500 hover:text-neutral-900 dark:hover:text-white", focusRing)}
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      {downloading && group && <GroupProgress group={group} compact onCancel={() => void cancelGroup(group.groupId)} />}
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
    </div>
  );
}
