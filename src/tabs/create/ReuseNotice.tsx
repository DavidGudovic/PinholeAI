// The settings reused from a picture dropped on Create's results area (see reuseSettings.ts).
import { Check, X } from "lucide-react";
import { IconButton } from "../../components/ui";
import type { ReuseOutcome } from "./reuseSettings";

export function ReuseNotice({ outcome, onDismiss }: { outcome: ReuseOutcome; onDismiss: () => void }) {
  return (
    <div className="pinhole-pop rounded-xl border border-neutral-200 bg-neutral-50 p-3 text-sm dark:border-neutral-800 dark:bg-neutral-900" role="status">
      <div className="flex items-start gap-2">
        <Check className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600" />
        <div className="min-w-0 flex-1 space-y-1">
          <div className="font-medium">Settings reused from the picture</div>
          <p className="text-xs text-neutral-600 dark:text-neutral-400">
            {outcome.modelSelected
              ? `Made with ${outcome.madeWith ?? "this model"}, which is selected now.`
              : outcome.madeWith
                ? `Made with ${outcome.madeWith}, which isn’t installed here. ${outcome.modelName ?? "Your model"} is kept.`
                : `${outcome.modelName ?? "Your model"} is kept.`}
          </p>
          {outcome.applied.length > 0 && <p className="text-xs text-neutral-600 dark:text-neutral-400">{outcome.applied.join(" · ")}</p>}
          {outcome.skipped.map((s) => (
            <p key={s.what} className="text-xs text-neutral-500">
              {s.what}: {s.why}
            </p>
          ))}
          <p className="text-xs text-neutral-500">Pictures never hold the prompt, so type it again. The picture wasn’t kept.</p>
        </div>
        <IconButton label="Dismiss" size="sm" onClick={onDismiss} className="-mt-1 -mr-1">
          <X className="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  );
}
