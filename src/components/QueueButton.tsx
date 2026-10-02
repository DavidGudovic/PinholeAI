// The "N waiting" button next to Generate / Apply edit: lists queued jobs (generations, edits, upscales), each removable.
import { ImageUp, ListOrdered, Sparkles, WandSparkles, X } from "lucide-react";
import { useActions } from "../lib/state/AppProvider";
import { useAppState } from "../lib/state/store";
import { IconButton, Popover, cx, focusRing } from "./ui";

export function QueueButton() {
  const queue = useAppState((s) => s.queue);
  const actions = useActions();
  if (!queue.length) return null;
  return (
    <Popover
      align="end"
      width={320}
      trigger={(p) => (
        <button
          {...p}
          type="button"
          aria-label={`${queue.length} waiting in the queue`}
          className={cx(
            "inline-flex h-11 shrink-0 items-center gap-1.5 rounded-lg border border-neutral-200 bg-white px-3 text-sm font-medium text-neutral-800 shadow-xs hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:hover:bg-neutral-700",
            focusRing,
          )}
        >
          <ListOrdered className="h-4 w-4" />
          {queue.length}
        </button>
      )}
    >
      <div className="px-2.5 pt-1.5 pb-1 text-xs font-medium text-neutral-500">Waiting, in order</div>
      <ul aria-label="Queue">
        {queue.map((q, i) => (
          <li key={q.id} className="flex items-center gap-2 rounded-lg px-2.5 py-1.5 hover:bg-neutral-100 dark:hover:bg-neutral-800">
            <span className="w-4 shrink-0 text-right text-xs tabular-nums text-neutral-400">{i + 1}</span>
            {q.kind === "upscale" || q.kind === "editUpscale" ? (
              <ImageUp className="h-3.5 w-3.5 shrink-0 text-neutral-500" aria-label="Upscale" />
            ) : q.kind === "edit" ? (
              <WandSparkles className="h-3.5 w-3.5 shrink-0 text-neutral-500" aria-label="Edit" />
            ) : (
              <Sparkles className="h-3.5 w-3.5 shrink-0 text-neutral-500" aria-label="Create" />
            )}
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm">{q.label}</div>
              <div className="truncate text-xs text-neutral-500">{q.detail}</div>
            </div>
            <IconButton label="Remove from queue" size="sm" onClick={() => actions.removeQueued(q.id)}>
              <X className="h-4 w-4" />
            </IconButton>
          </li>
        ))}
      </ul>
    </Popover>
  );
}
