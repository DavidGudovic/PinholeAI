import type { Job, JobKind } from "../lib/state/model";
import { useElapsed } from "../lib/state/hooks";
import { useAppState } from "../lib/state/store";
import { Button, ProgressBar, Spinner } from "./ui";

/** Plain-words status for a running job. */
export function jobStatusText(job: Job, elapsed: number): string {
  const p = job.progress;
  const verb = job.kind === "upscale" || job.kind === "editUpscale" ? "Upscaling" : job.kind === "edit" ? "Editing" : "Creating";
  if (!p) return `${verb}…`;
  switch (p.phase) {
    case "loadingModel":
      // step/totalSteps = model tensors loaded / total.
      if (p.step != null && p.totalSteps) return `Loading ${p.modelLabel ?? "the model"}… ${Math.min(100, Math.round((p.step / p.totalSteps) * 100))}%`;
      return `Loading ${p.modelLabel ?? "the model"}… (~10–30 s)`;
    case "queued":
      return p.queuePosition ? `Waiting in line (#${p.queuePosition})…` : "Waiting in line…";
    case "generating":
      if (p.step != null && p.totalSteps) return `${verb} · step ${Math.min(p.step, p.totalSteps)} of ${p.totalSteps}`;
      return `${verb}… ${elapsed} s`;
    case "done":
      return "Finishing…";
    case "cancelled":
      return "Stopping…";
    default:
      return `${verb}…`;
  }
}

/**
 * JobProgress for the running job when it is one of `kinds`. Only this component
 * subscribes to the job object, so the ~3 progress ticks a second re-render the
 * progress card and not the whole sidebar around it.
 */
export function LiveJobProgress({ kinds, onCancel, cancelling }: { kinds: readonly JobKind[]; onCancel: () => void; cancelling?: boolean }) {
  // null for another tab's job, so its ticks don't re-render this one either.
  const job = useAppState((s) => (s.job && kinds.includes(s.job.kind) ? s.job : null));
  if (!job) return null;
  return <JobProgress job={job} onCancel={onCancel} cancelling={cancelling} />;
}

export function JobProgress({ job, onCancel, cancelling }: { job: Job; onCancel: () => void; cancelling?: boolean }) {
  const elapsed = useElapsed(job.startedAt, true);
  const p = job.progress;
  const determinate = (p?.phase === "generating" || p?.phase === "loadingModel") && p.step != null && !!p.totalSteps;
  return (
    <div className="rounded-xl border border-amber-200 bg-amber-50/70 p-3 dark:border-amber-500/20 dark:bg-amber-500/5" role="status" aria-live="polite">
      <div className="flex items-center gap-3">
        <Spinner className="h-4 w-4 text-amber-600 dark:text-amber-400" />
        <div className="min-w-0 flex-1 text-sm font-medium text-amber-950 dark:text-amber-100">
          <span className="block truncate">{jobStatusText(job, elapsed)}</span>
        </div>
        <span className="shrink-0 text-xs tabular-nums text-amber-900/60 dark:text-amber-200/60">{elapsed} s</span>
        <Button size="sm" variant="secondary" onClick={onCancel} disabled={cancelling}>
          {cancelling ? "Stopping…" : "Cancel"}
        </Button>
      </div>
      {p?.note && (
        <p className="mt-1.5 pl-7 text-xs text-amber-900/80 dark:text-amber-200/80" role="note">
          {p.note}
        </p>
      )}
      <ProgressBar className="mt-2.5" value={p?.step ?? 0} max={p?.totalSteps || 1} indeterminate={!determinate} />
    </div>
  );
}
