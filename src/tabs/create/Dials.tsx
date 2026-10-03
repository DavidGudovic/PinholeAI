// Simple dials (SPEC §5.1): Shape · Quality · Stick to prompt · How many · Keep this look.
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Lock } from "lucide-react";
import { Segmented, Slider, Toggle, cx, focusRing } from "../../components/ui";
import { DEFAULT_SHAPE_SIZES, SHAPE_LABEL, defaultStickPosition, stickValue } from "../../lib/paste/map";
import type { FamilyUi, Quality, Shape } from "../../lib/types";
import { useAppState, useDispatch } from "../../lib/state/store";

export const FALLBACK_SHAPES: Record<Shape, [number, number]> = DEFAULT_SHAPE_SIZES;
const SHAPES: Shape[] = ["square", "portrait", "landscape", "wide"];
const QUALITIES: Quality[] = ["fast", "balanced", "best"];
export const qualityIndex = (q: Quality) => QUALITIES.indexOf(q);

export function DialRow({ label, children, htmlFor }: { label: ReactNode; children: ReactNode; htmlFor?: string }) {
  return (
    <div className="grid grid-cols-[7rem_minmax(0,1fr)] items-center gap-3">
      <label htmlFor={htmlFor} className="text-sm text-neutral-600 dark:text-neutral-400">
        {label}
      </label>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

export function ShapeChips({ value, onChange, shapes }: { value: Shape; onChange: (s: Shape) => void; shapes: Record<string, [number, number]> }) {
  return (
    <div role="radiogroup" aria-label="Shape" className="grid grid-cols-4 gap-1.5">
      {SHAPES.map((s) => {
        const [w, h] = shapes[s] ?? FALLBACK_SHAPES[s];
        return <ShapeChip key={s} active={s === value} w={w} h={h} title={`${w}×${h}`} label={SHAPE_LABEL[s]} onClick={() => onChange(s)} />;
      })}
    </div>
  );
}

/** One shape chip (a radio): an outline of `w`×`h` above the label. `inset` draws a smaller box inside it. */
export function ShapeChip({
  active,
  w,
  h,
  label,
  title,
  onClick,
  disabled,
  inset,
}: {
  active: boolean;
  w: number;
  h: number;
  label: ReactNode;
  title?: string;
  onClick: () => void;
  disabled?: boolean;
  inset?: boolean;
}) {
  const labelRef = useFitText(label, active);
  const max = 18;
  const iw = w >= h ? max : Math.round((max * w) / h);
  const ih = h >= w ? max : Math.round((max * h) / w);
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      title={title}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "flex h-14 min-w-0 flex-col items-center justify-center gap-1 rounded-lg border text-xs transition-colors disabled:cursor-not-allowed disabled:opacity-40",
        focusRing,
        active
          ? "border-amber-500 bg-amber-50 font-medium text-amber-950 dark:border-amber-500/70 dark:bg-amber-500/10 dark:text-amber-100"
          : "border-neutral-200 bg-white text-neutral-600 enabled:hover:border-neutral-300 enabled:hover:text-neutral-900 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-400 dark:enabled:hover:border-neutral-600 dark:enabled:hover:text-white",
      )}
    >
      <span className="flex h-5 w-5 items-center justify-center" aria-hidden>
        <span
          className={cx(
            "flex items-center justify-center rounded-[3px] border-[1.5px]",
            active ? "border-amber-600 bg-amber-500/15 dark:border-amber-400" : "border-current opacity-70",
            inset && "border-dashed",
          )}
          style={{ width: iw, height: ih }}
        >
          {inset && <span className="rounded-[2px] border-[1.5px] border-current" style={{ width: iw / 2, height: ih / 2 }} />}
        </span>
      </span>
      <span ref={labelRef} className="max-w-full truncate">{label}</span>
    </button>
  );
}

/** Shrinks a one-line label's text a little when it is wider than its box (a wide system font in a narrow column). */
function useFitText(label: ReactNode, active: boolean) {
  const ref = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const fit = () => {
      el.style.fontSize = "";
      if (el.scrollWidth <= el.clientWidth) return;
      const size = parseFloat(getComputedStyle(el).fontSize);
      el.style.fontSize = `${Math.max(10, Math.floor(size * (el.clientWidth / el.scrollWidth) * 10) / 10)}px`;
    };
    fit();
    const ro = typeof ResizeObserver !== "undefined" ? new ResizeObserver(fit) : null;
    ro?.observe(el.parentElement ?? el);
    return () => ro?.disconnect();
    // A selected chip's label is bold, so wider: measure again when it is picked.
  }, [label, active]);
  return ref;
}

export function Dials({ ui }: { ui: FamilyUi | null }) {
  const c = useAppState((s) => s.create);
  const selected = useAppState((s) => s.results.find((r) => r.id === s.selectedResultId) ?? null);
  const dispatch = useDispatch();
  const shapes = ui?.shapes ?? FALLBACK_SHAPES;
  const steps = ui?.qualitySteps;
  const stickPos = c.stick ?? defaultStickPosition(ui);
  const stickVal = ui ? stickValue(ui, stickPos) : null;
  const seedLocked = c.fineTune.seed != null;
  const canLock = seedLocked || !!selected;

  return (
    <div className="space-y-3.5">
      <DialRow label="Shape">
        <ShapeChips value={c.shape} shapes={shapes} onChange={(v) => dispatch({ type: "setDial", dial: "shape", value: v })} />
      </DialRow>
      <DialRow label="Quality">
        <Segmented
          ariaLabel="Quality"
          stretch
          value={c.quality}
          onChange={(v) => dispatch({ type: "setDial", dial: "quality", value: v })}
          options={QUALITIES.map((q, i) => ({
            value: q,
            label: q === "fast" ? "Fast" : q === "balanced" ? "Balanced" : "Best",
            title: steps ? `${steps[i]} steps${q === "best" && ui?.hiresAtBest ? " + hires fix" : ""}` : undefined,
          }))}
        />
      </DialRow>
      {(!ui || ui.showStick) && (
        <DialRow label="Stick to prompt">
          <Slider
            ariaLabel="Stick to prompt"
            ariaValueText={stickVal != null ? `${ui?.stickMapsTo === "guidance" ? "Guidance" : "CFG"} ${stickVal}` : undefined}
            value={stickPos}
            onChange={(v) => dispatch({ type: "setDial", dial: "stick", value: v })}
            left="Loose"
            right="Strict"
          />
        </DialRow>
      )}
      <DialRow label="How many">
        <Segmented
          ariaLabel="How many"
          value={c.count}
          onChange={(v) => dispatch({ type: "setDial", dial: "count", value: v })}
          options={[
            { value: 1 as const, label: "1" },
            { value: 2 as const, label: "2" },
            { value: 4 as const, label: "4" },
          ]}
        />
      </DialRow>
      <DialRow label="Keep this look">
        <span title={canLock ? undefined : "Generate and select an image first"} className="inline-flex">
          <Toggle
            checked={seedLocked}
            disabled={!canLock}
            onChange={(on) => dispatch({ type: "keepLook", on })}
            label={
              <span className="inline-flex items-center gap-1 text-xs text-neutral-500">
                {seedLocked ? (
                  <>
                    <Lock className="h-3 w-3 text-amber-600" /> Seed {c.fineTune.seed}
                  </>
                ) : canLock ? (
                  "Reuse the selected image’s seed"
                ) : (
                  "Select an image first"
                )}
              </span>
            }
          />
        </span>
      </DialRow>
    </div>
  );
}
