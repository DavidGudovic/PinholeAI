// Shared UI primitives (Tailwind). Orchestrator-provided starting set; frontend
// A owns this folder from now on — frontend B may import but not edit.
import { useEffect, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import type { CoreError, Fit, VramNeed } from "../../lib/types";
import { formatGb } from "../../lib/format";

type Variant = "primary" | "secondary" | "ghost" | "danger";
const variants: Record<Variant, string> = {
  primary: "bg-amber-500 text-neutral-950 hover:bg-amber-400 disabled:bg-amber-500/50",
  secondary:
    "bg-neutral-200 text-neutral-900 hover:bg-neutral-300 dark:bg-neutral-800 dark:text-neutral-100 dark:hover:bg-neutral-700",
  ghost: "text-neutral-700 hover:bg-neutral-200 dark:text-neutral-300 dark:hover:bg-neutral-800",
  danger: "bg-red-600 text-white hover:bg-red-500",
};

export function Button({
  variant = "secondary",
  size = "md",
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "sm" | "md" | "lg" }) {
  const sizes = { sm: "px-2.5 py-1 text-xs", md: "px-3.5 py-2 text-sm", lg: "px-5 py-2.5 text-base" };
  return (
    <button
      className={`inline-flex items-center justify-center gap-1.5 rounded-lg font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${variants[variant]} ${sizes[size]} ${className}`}
      {...rest}
    />
  );
}

export function Card({ className = "", children }: { className?: string; children: ReactNode }) {
  return (
    <div className={`rounded-xl border border-neutral-200 bg-white p-4 shadow-sm dark:border-neutral-800 dark:bg-neutral-900 ${className}`}>
      {children}
    </div>
  );
}

export function Badge({ tone = "neutral", children }: { tone?: "neutral" | "green" | "amber" | "red" | "blue"; children: ReactNode }) {
  const tones = {
    neutral: "bg-neutral-200 text-neutral-700 dark:bg-neutral-800 dark:text-neutral-300",
    green: "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-300",
    amber: "bg-amber-100 text-amber-800 dark:bg-amber-900/50 dark:text-amber-300",
    red: "bg-red-100 text-red-800 dark:bg-red-900/50 dark:text-red-300",
    blue: "bg-sky-100 text-sky-800 dark:bg-sky-900/50 dark:text-sky-300",
  };
  return <span className={`inline-flex items-center rounded-md px-1.5 py-0.5 text-xs font-medium ${tones[tone]}`}>{children}</span>;
}

/** "Needs ~X GB VRAM" + Fits / Tight / Too big (SPEC §6.2). */
export function VramBadge({ vram, fit }: { vram: VramNeed | null; fit: Fit | null }) {
  if (!vram) return null;
  const label = fit === "fits" ? "Fits" : fit === "tight" ? "Tight" : fit === "tooBig" ? "Too big" : null;
  const tone = fit === "fits" ? "green" : fit === "tight" ? "amber" : "red";
  return (
    <span className="inline-flex items-center gap-1.5 text-xs text-neutral-600 dark:text-neutral-400">
      Needs ~{formatGb(vram.gb)} VRAM{vram.estimate ? " (estimate)" : ""}
      {label && <Badge tone={tone}>{label}</Badge>}
    </span>
  );
}

export function Spinner({ className = "h-4 w-4" }: { className?: string }) {
  return <span className={`inline-block animate-spin rounded-full border-2 border-current border-t-transparent ${className}`} />;
}

export function ProgressBar({ value, max = 1, indeterminate = false }: { value?: number; max?: number; indeterminate?: boolean }) {
  const pct = indeterminate || !max ? 100 : Math.min(100, Math.max(0, ((value ?? 0) / max) * 100));
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800">
      <div className={`h-full rounded-full bg-amber-500 transition-all ${indeterminate ? "animate-pulse" : ""}`} style={{ width: `${pct}%` }} />
    </div>
  );
}

export function Toggle({ checked, onChange, label, disabled }: { checked: boolean; onChange: (v: boolean) => void; label?: ReactNode; disabled?: boolean }) {
  return (
    <label className={`inline-flex items-center gap-2 text-sm ${disabled ? "opacity-60" : "cursor-pointer"}`}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative h-5 w-9 rounded-full transition-colors ${checked ? "bg-amber-500" : "bg-neutral-300 dark:bg-neutral-700"}`}
      >
        <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-all ${checked ? "left-4.5" : "left-0.5"}`} />
      </button>
      {label}
    </label>
  );
}

export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  size = "md",
}: {
  options: { value: T; label: ReactNode; title?: string }[];
  value: T;
  onChange: (v: T) => void;
  size?: "sm" | "md";
}) {
  return (
    <div className="inline-flex rounded-lg bg-neutral-200 p-0.5 dark:bg-neutral-800">
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          title={o.title}
          onClick={() => onChange(o.value)}
          className={`rounded-md ${size === "sm" ? "px-2 py-0.5 text-xs" : "px-3 py-1 text-sm"} transition-colors ${
            o.value === value
              ? "bg-white text-neutral-900 shadow-sm dark:bg-neutral-600 dark:text-white"
              : "text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-white"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Slider({
  value,
  onChange,
  min = 0,
  max = 1,
  step = 0.01,
  left,
  right,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  left?: ReactNode;
  right?: ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 text-xs text-neutral-500">
      {left && <span>{left}</span>}
      <input
        type="range"
        className="w-full accent-amber-500"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      {right && <span>{right}</span>}
    </div>
  );
}

/** Centered modal. */
export function Dialog({ open, onClose, title, children, footer, wide }: { open: boolean; onClose: () => void; title: ReactNode; children: ReactNode; footer?: ReactNode; wide?: boolean }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        className={`max-h-[90vh] w-full ${wide ? "max-w-3xl" : "max-w-lg"} overflow-auto rounded-2xl bg-white shadow-xl dark:bg-neutral-900`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="border-b border-neutral-200 px-5 py-3 text-base font-semibold dark:border-neutral-800">{title}</div>
        <div className="px-5 py-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-neutral-200 px-5 py-3 dark:border-neutral-800">{footer}</div>}
      </div>
    </div>
  );
}

/** Right-side panel (Settings, Fine-tune on narrow screens). */
export function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title: ReactNode; children: ReactNode }) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-40 flex justify-end bg-black/40" onMouseDown={onClose}>
      <div className="h-full w-full max-w-md overflow-auto bg-white shadow-xl dark:bg-neutral-900" onMouseDown={(e) => e.stopPropagation()}>
        <div className="sticky top-0 flex items-center justify-between border-b border-neutral-200 bg-white px-5 py-3 dark:border-neutral-800 dark:bg-neutral-900">
          <span className="text-base font-semibold">{title}</span>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="Close">
            ✕
          </Button>
        </div>
        <div className="px-5 py-4">{children}</div>
      </div>
    </div>
  );
}

/** Plain-language error with the technical output behind a "Details" toggle. */
export function ErrorNotice({ error, onDismiss }: { error: CoreError; onDismiss?: () => void }) {
  const [show, setShow] = useState(false);
  return (
    <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
      <div className="flex items-start justify-between gap-3">
        <p>{error.message}</p>
        {onDismiss && (
          <button className="text-red-700 hover:underline dark:text-red-300" onClick={onDismiss}>
            Dismiss
          </button>
        )}
      </div>
      {error.details && (
        <>
          <button className="mt-1 text-xs underline" onClick={() => setShow((s) => !s)}>
            {show ? "Hide details" : "Details"}
          </button>
          {show && <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded bg-black/5 p-2 text-xs dark:bg-white/5">{error.details}</pre>}
        </>
      )}
    </div>
  );
}

export function Field({ label, hint, children }: { label: ReactNode; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block space-y-1">
      <span className="text-sm font-medium text-neutral-700 dark:text-neutral-300">{label}</span>
      {children}
      {hint && <span className="block text-xs text-neutral-500">{hint}</span>}
    </label>
  );
}

export const inputClass =
  "w-full rounded-lg border border-neutral-300 bg-white px-3 py-2 text-sm outline-none focus:border-amber-500 focus:ring-2 focus:ring-amber-500/30 dark:border-neutral-700 dark:bg-neutral-950";
