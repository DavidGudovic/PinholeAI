import { X } from "lucide-react";
import { useAppState, useDispatch } from "../lib/state/store";
import { cx, focusRing } from "./ui";

/** Bottom-center notifications ("Saved to …  Show folder", "Copied"). */
export function Toasts() {
  const toasts = useAppState((s) => s.toasts);
  const dispatch = useDispatch();
  if (!toasts.length) return null;
  return (
    <div aria-live="polite" className="pointer-events-none fixed inset-x-0 bottom-5 z-[70] flex flex-col items-center gap-2 px-4">
      {toasts.map((t) => (
        <div
          key={t.id}
          role="status"
          className={cx(
            "pinhole-pop pointer-events-auto flex max-w-xl items-center gap-3 rounded-xl px-4 py-2.5 text-sm shadow-lg ring-1",
            t.tone === "error"
              ? "bg-red-600 text-white ring-red-700"
              : "bg-neutral-900 text-neutral-100 ring-black/10 dark:bg-neutral-100 dark:text-neutral-900 dark:ring-white/10",
          )}
        >
          <span className="min-w-0 [overflow-wrap:anywhere]">{t.text}</span>
          {t.action && (
            <button
              type="button"
              className={cx("shrink-0 rounded font-semibold text-amber-400 hover:underline dark:text-amber-700", focusRing)}
              onClick={() => {
                t.action!.run();
                dispatch({ type: "dismissToast", id: t.id });
              }}
            >
              {t.action.label}
            </button>
          )}
          <button
            type="button"
            aria-label="Dismiss"
            className={cx("-mr-1 shrink-0 rounded opacity-60 hover:opacity-100", focusRing)}
            onClick={() => dispatch({ type: "dismissToast", id: t.id })}
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      ))}
    </div>
  );
}
