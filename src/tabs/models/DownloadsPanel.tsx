// Active (and recently finished) download groups with Cancel.
import { CircleCheck, CircleX, Download, X } from "lucide-react";
import { GroupProgress } from "./controls";
import { IconButton } from "../../components/ui";
import { cancelGroup, hideFinished, hideGroup, useVisibleDownloads } from "./lib/downloads";
import { isActive } from "./lib/words";

export function DownloadsPanel() {
  const groups = useVisibleDownloads();
  if (!groups.length) return null;
  const active = groups.filter(isActive);
  const finished = groups.length - active.length;
  const list = groups.slice().reverse();
  return (
    <section aria-label="Downloads" className="rounded-xl border border-neutral-200 bg-white shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
      <header className="flex items-center justify-between gap-3 border-b border-neutral-100 px-4 py-2 dark:border-neutral-800">
        <span className="inline-flex items-center gap-2 text-sm font-medium">
          <Download className="h-4 w-4 text-amber-500" />
          Downloads
          {active.length > 0 && <span className="text-neutral-500">· {active.length} active</span>}
        </span>
        {finished > 0 && (
          <button type="button" onClick={hideFinished} className="text-xs text-neutral-500 hover:text-neutral-900 hover:underline dark:hover:text-neutral-100">
            Clear finished
          </button>
        )}
      </header>
      <ul className="max-h-64 divide-y divide-neutral-100 overflow-y-auto dark:divide-neutral-800">
        {list.map((g) => (
          <li key={g.groupId} className="px-4 py-2.5">
            <div className="flex items-center justify-between gap-3">
              <span className="truncate text-sm font-medium text-neutral-800 dark:text-neutral-200">{g.label}</span>
              {!isActive(g) && (
                <IconButton size="sm" label="Remove from list" onClick={() => hideGroup(g.groupId)}>
                  <X className="h-3.5 w-3.5" />
                </IconButton>
              )}
            </div>
            {isActive(g) ? (
              <div className="mt-1">
                <GroupProgress group={g} onCancel={() => void cancelGroup(g.groupId)} />
              </div>
            ) : g.state === "done" ? (
              <p className="inline-flex items-center gap-1.5 text-xs text-emerald-700 dark:text-emerald-400">
                <CircleCheck className="h-3.5 w-3.5" /> Installed — ready to use
              </p>
            ) : g.state === "failed" ? (
              <p className="inline-flex items-start gap-1.5 text-xs text-red-700 dark:text-red-400">
                <CircleX className="mt-px h-3.5 w-3.5 shrink-0" /> {g.error ?? "The download failed. Try again from where you started it."}
              </p>
            ) : (
              <p className="text-xs text-neutral-500">Cancelled</p>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
