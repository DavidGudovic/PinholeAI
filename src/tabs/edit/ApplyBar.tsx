// The bottom of the Edit panel: job progress, the Apply button and queue, status lines and errors.
import { ListPlus, WandSparkles } from "lucide-react";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { LiveJobProgress } from "../../components/JobProgress";
import { QueueButton } from "../../components/QueueButton";
import { Button, Kbd } from "../../components/ui";
import type { CoreError } from "../../lib/types";
import type { EditMode } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";

const EDIT_JOBS = ["edit", "editUpscale", "editMore"] as const;

export function ApplyBar({
  mode,
  fixing,
  extending,
  painted,
  hasImage,
  job,
  myJob,
  queues,
  canRun,
  pictures = 1,
  onRun,
  cancelling,
  onCancel,
  error,
  onDismiss,
  onRetry,
}: {
  mode: EditMode;
  fixing: boolean;
  extending: boolean;
  painted: boolean;
  hasImage: boolean;
  job: boolean;
  myJob: boolean;
  queues: boolean;
  canRun: boolean;
  /** How many pictures Apply edits (with "Also apply to…"). */
  pictures?: number;
  onRun: () => void;
  cancelling: boolean;
  onCancel: () => Promise<void>;
  error: CoreError | null;
  onDismiss: () => void;
  onRetry: () => void;
}) {
  return (
    <div className="shrink-0 space-y-2 border-t border-neutral-200 px-5 py-4 dark:border-neutral-800">
      {myJob && (
        <LiveJobProgress
          kinds={EDIT_JOBS}
          cancelling={cancelling}
          onCancel={onCancel}
        />
      )}
      <div className="flex gap-2">
        <Button
          variant="primary"
          size="lg"
          className="min-w-0 flex-1"
          disabled={!canRun}
          onClick={onRun}
        >
          {queues ? (
            <ListPlus className="h-4 w-4" />
          ) : (
            <WandSparkles className="h-4 w-4" />
          )}
          {queues
            ? "Add to queue"
            : mode === "instruction"
              ? pictures > 1
                ? `Apply edit to ${pictures} pictures`
                : "Apply edit"
              : fixing
                ? painted
                  ? "Fix details"
                  : "Add detail"
                : extending
                  ? "Extend"
                  : pictures > 1
                    ? `Restyle ${pictures} pictures`
                    : "Restyle"}
          <span className="ml-1 inline-flex gap-0.5 opacity-70">
            <Kbd>{modKey}</Kbd>
            <Kbd>Enter</Kbd>
          </span>
        </Button>
        <QueueButton />
      </div>
      {!hasImage && !myJob && (
        <p className="text-center text-xs text-neutral-500">
          Add an image to start.
        </p>
      )}
      {job && !myJob && (
        <p className="text-center text-xs text-neutral-500">
          Busy creating. Edits wait for it to finish.
        </p>
      )}
      {error && (
        <ErrorWithFix
          error={error}
          onDismiss={onDismiss}
          onRetry={onRetry}
        />
      )}
    </div>
  );
}
