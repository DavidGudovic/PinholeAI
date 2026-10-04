// The row of edit steps under the picture; click one to go back to it.
import { memo } from "react";
import { ArrowRight } from "lucide-react";
import { cx, focusRing } from "../../components/ui";
import { thumbSrc } from "../../lib/state/images";
import type { Action, EditNode, ImgRef } from "../../lib/state/model";

// Only the history's own props, so typing in Edit's text boxes doesn't re-render it.
export const EditHistory = memo(function EditHistory({
  chain,
  index,
  images,
  locked,
  dispatch,
}: {
  chain: EditNode[];
  index: number;
  images: Record<string, ImgRef>;
  locked: boolean;
  dispatch: (a: Action) => void;
}) {
  return (
    <div className="shrink-0 border-t border-neutral-200 bg-white/60 px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900/40">
      <ol
        className="flex items-center gap-1.5 overflow-x-auto pb-1"
        aria-label="Edit history"
      >
        {chain.map((n, i) => {
          const img = images[n.imageId];
          return (
            <li
              key={n.imageId}
              className="flex shrink-0 items-center gap-1.5"
            >
              {i > 0 && (
                <ArrowRight
                  className="h-3.5 w-3.5 text-neutral-400"
                  aria-hidden
                />
              )}
              <button
                type="button"
                aria-current={i === index ? "step" : undefined}
                disabled={locked && i !== index}
                onClick={() => dispatch({ type: "editGoto", index: i })}
                className={cx(
                  "group flex flex-col items-center gap-1 rounded-lg p-1 disabled:cursor-not-allowed disabled:opacity-50",
                  focusRing,
                  i === index
                    ? "bg-amber-50 dark:bg-amber-500/10"
                    : "hover:bg-neutral-100 dark:hover:bg-neutral-800",
                )}
              >
                {img && (
                  <img
                    src={thumbSrc(img)}
                    alt=""
                    decoding="async"
                    loading="lazy"
                    className={cx(
                      "h-14 w-14 rounded-md object-cover ring-2",
                      i === index
                        ? "ring-amber-500"
                        : "ring-transparent",
                    )}
                    draggable={false}
                  />
                )}
                <span
                  className={cx(
                    "text-[11px]",
                    i === index
                      ? "font-medium text-amber-900 dark:text-amber-200"
                      : "text-neutral-500",
                  )}
                >
                  {n.label}
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
});
