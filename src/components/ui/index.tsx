// Shared UI primitives (Tailwind).
// Keep existing exports and props backwards compatible.
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ComponentProps,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type Ref,
  type RefObject,
  type TextareaHTMLAttributes,
} from "react";
import { createPortal } from "react-dom";
import { ChevronDown, CircleAlert, X } from "lucide-react";
import type { CoreError, Fit, VramNeed } from "../../lib/types";
import { fitWords } from "../../lib/format";

export const cx = (...c: (string | false | null | undefined)[]) => c.filter(Boolean).join(" ");

/** Keyboard focus ring used by every interactive primitive. */
export const focusRing =
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-500/70 focus-visible:ring-offset-2 focus-visible:ring-offset-white dark:focus-visible:ring-offset-neutral-900";

type Variant = "primary" | "secondary" | "ghost" | "danger";
const variants: Record<Variant, string> = {
  primary: "bg-amber-500 text-neutral-950 shadow-sm hover:bg-amber-400 active:bg-amber-500",
  secondary:
    "border border-neutral-200 bg-white text-neutral-800 shadow-xs hover:bg-neutral-50 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100 dark:hover:bg-neutral-700",
  ghost: "text-neutral-700 hover:bg-neutral-200/70 dark:text-neutral-300 dark:hover:bg-neutral-800",
  danger: "bg-red-600 text-white shadow-sm hover:bg-red-500",
};

export function Button({
  variant = "secondary",
  size = "md",
  className = "",
  type = "button",
  ...rest
}: ComponentProps<"button"> & { variant?: Variant; size?: "sm" | "md" | "lg" }) {
  const sizes = { sm: "h-7 px-2.5 text-xs", md: "h-9 px-3.5 text-sm", lg: "h-11 px-5 text-base" };
  return (
    <button
      type={type}
      className={cx(
        "inline-flex shrink-0 select-none items-center justify-center gap-1.5 whitespace-nowrap rounded-lg font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-60",
        focusRing,
        variants[variant],
        sizes[size],
        className,
      )}
      {...rest}
    />
  );
}

/** Square icon-only button. `label` becomes aria-label and tooltip. */
export function IconButton({
  label,
  size = "md",
  variant = "ghost",
  className = "",
  children,
  ...rest
}: ComponentProps<"button"> & { label: string; size?: "sm" | "md"; variant?: Variant }) {
  const sizes = { sm: "h-7 w-7", md: "h-9 w-9" };
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex shrink-0 items-center justify-center rounded-lg transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        focusRing,
        variants[variant],
        sizes[size],
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Card({ className = "", children }: { className?: string; children: ReactNode }) {
  return (
    <div className={cx("rounded-xl border border-neutral-200 bg-white p-4 shadow-xs dark:border-neutral-800 dark:bg-neutral-900", className)}>
      {children}
    </div>
  );
}

export function Badge({ tone = "neutral", title, children }: { tone?: "neutral" | "green" | "amber" | "red" | "blue"; title?: string; children: ReactNode }) {
  const tones = {
    neutral: "bg-neutral-100 text-neutral-700 border-neutral-200 dark:bg-neutral-800 dark:text-neutral-300 dark:border-neutral-700",
    green: "bg-emerald-50 text-emerald-800 border-emerald-200 dark:bg-emerald-950/60 dark:text-emerald-300 dark:border-emerald-900",
    amber: "bg-amber-50 text-amber-800 border-amber-200 dark:bg-amber-950/60 dark:text-amber-300 dark:border-amber-900",
    red: "bg-red-50 text-red-800 border-red-200 dark:bg-red-950/60 dark:text-red-300 dark:border-red-900",
    blue: "bg-sky-50 text-sky-800 border-sky-200 dark:bg-sky-950/60 dark:text-sky-300 dark:border-sky-900",
  };
  return (
    // A 1px border inside the same outer size (not an inset ring: that is a box-shadow, which
    // WebKitGTK repaints slowly when many badges scroll by).
    <span title={title} className={cx("inline-flex items-center whitespace-nowrap rounded-md border px-[5px] py-px text-[11px] font-medium leading-4", tones[tone])}>
      {children}
    </span>
  );
}

/** "Needs ~X GB VRAM" + Fits / Tight / Too big (SPEC §6.2). */
/** "Needs ~X GB VRAM" + Fits / Tight / Too big — or, without a usable GPU (`vram.onCpu`), "Needs ~X GB memory" + Slow / Too big. */
export function VramBadge({ vram, fit, compact = false }: { vram: VramNeed | null; fit: Fit | null; compact?: boolean }) {
  if (!vram) return null;
  const w = fitWords(vram, fit);
  return (
    <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-neutral-500 dark:text-neutral-400" title={w.title}>
      {compact ? w.short : w.need}
      {w.badge && <Badge tone={w.tone}>{w.badge}</Badge>}
    </span>
  );
}

export function Spinner({ className = "h-4 w-4" }: { className?: string }) {
  return <span aria-hidden className={cx("inline-block shrink-0 animate-spin rounded-full border-2 border-current border-t-transparent", className)} />;
}

export function ProgressBar({ value, max = 1, indeterminate = false, className = "" }: { value?: number; max?: number; indeterminate?: boolean; className?: string }) {
  const pct = indeterminate || !max ? 100 : Math.min(100, Math.max(0, ((value ?? 0) / max) * 100));
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : Math.round(pct)}
      className={cx("relative h-1.5 w-full overflow-hidden rounded-full bg-neutral-200 dark:bg-neutral-800", className)}
    >
      {indeterminate ? (
        <div className="pinhole-indeterminate absolute inset-y-0 w-1/3 rounded-full bg-amber-500" />
      ) : (
        <div className="h-full rounded-full bg-amber-500 transition-[width] duration-300" style={{ width: `${pct}%` }} />
      )}
    </div>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  disabled,
  hint,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label?: ReactNode;
  disabled?: boolean;
  hint?: ReactNode;
}) {
  return (
    <label className={cx("inline-flex items-center gap-2.5 text-sm", disabled ? "opacity-60" : "cursor-pointer")}>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cx(
          "relative h-5 w-9 shrink-0 rounded-full transition-colors disabled:cursor-not-allowed",
          focusRing,
          checked ? "bg-amber-500" : "bg-neutral-300 dark:bg-neutral-700",
        )}
      >
        <span className={cx("absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-all", checked ? "left-4.5" : "left-0.5")} />
      </button>
      {(label || hint) && (
        <span className="min-w-0">
          {label}
          {hint && <span className="block text-xs text-neutral-500">{hint}</span>}
        </span>
      )}
    </label>
  );
}

export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  size = "md",
  ariaLabel,
  disabled,
  stretch = false,
}: {
  options: { value: T; label: ReactNode; title?: string }[];
  value: T;
  onChange: (v: T) => void;
  size?: "sm" | "md";
  ariaLabel?: string;
  disabled?: boolean;
  /** Fill the available width with equal segments. */
  stretch?: boolean;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      className={cx("rounded-lg bg-neutral-200/70 p-0.5 dark:bg-neutral-800", stretch ? "flex w-full" : "inline-flex", disabled && "opacity-60")}
    >
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={String(o.value)}
            type="button"
            role="radio"
            aria-checked={active}
            title={o.title}
            disabled={disabled}
            onClick={() => onChange(o.value)}
            className={cx(
              "inline-flex items-center justify-center gap-1 whitespace-nowrap rounded-md font-medium transition-colors disabled:cursor-not-allowed",
              focusRing,
              stretch && "flex-1",
              size === "sm" ? "h-6 px-2 text-xs" : "h-7 px-3 text-sm",
              active
                ? "bg-white text-neutral-900 shadow-sm dark:bg-neutral-600 dark:text-white"
                : "text-neutral-600 hover:text-neutral-900 dark:text-neutral-400 dark:hover:text-white",
            )}
          >
            {o.label}
          </button>
        );
      })}
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
  ariaLabel,
  disabled,
  ariaValueText,
}: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  step?: number;
  left?: ReactNode;
  right?: ReactNode;
  ariaLabel?: string;
  disabled?: boolean;
  ariaValueText?: string;
}) {
  return (
    <div className={cx("flex items-center gap-2.5 text-xs text-neutral-500", disabled && "opacity-60")}>
      {left && <span className="shrink-0">{left}</span>}
      <input
        type="range"
        aria-label={ariaLabel}
        aria-valuetext={ariaValueText}
        disabled={disabled}
        className={cx("pinhole-range w-full min-w-0", focusRing)}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
      {right && <span className="shrink-0">{right}</span>}
    </div>
  );
}

/** Open overlays, most recently opened last: Escape closes only that one. */
const escapeStack: { close: () => void }[] = [];

function onEscapeKey(e: KeyboardEvent) {
  const top = escapeStack[escapeStack.length - 1];
  if (e.key !== "Escape" || !top) return;
  // Other window-level Escape handlers (e.g. a details page) wait for the next press.
  e.stopImmediatePropagation();
  top.close();
}

export function useEscape(open: boolean, onClose: () => void) {
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    if (!open) return;
    const entry = { close: () => close.current() };
    escapeStack.push(entry);
    if (escapeStack.length === 1) window.addEventListener("keydown", onEscapeKey, true);
    return () => {
      escapeStack.splice(escapeStack.indexOf(entry), 1);
      if (!escapeStack.length) window.removeEventListener("keydown", onEscapeKey, true);
    };
  }, [open]);
}

/**
 * Modal focus: on open, remember the focused element and move focus into the
 * panel (the first match of `first`, else the panel itself); on close, put it back.
 * `first` is a selector, or a function that picks the element (keep it stable across renders).
 * Tab and Shift+Tab wrap around inside the panel, so focus stays in it while it is open.
 */
export function useModalFocus(open: boolean, panel: RefObject<HTMLElement | null>, first?: string | ((panel: HTMLElement) => HTMLElement | null)) {
  useEffect(() => {
    if (!open) return;
    const prev = document.activeElement as HTMLElement | null;
    const t = setTimeout(() => {
      const root = panel.current;
      const el = !root || !first ? null : typeof first === "string" ? root.querySelector<HTMLElement>(first) : first(root);
      (el ?? root)?.focus();
    }, 0);
    const root = panel.current;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab" || !root) return;
      const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.closest("[hidden], [inert]"));
      if (!items.length) {
        e.preventDefault();
        root.focus();
        return;
      }
      const active = document.activeElement;
      const firstEl = items[0];
      const lastEl = items[items.length - 1];
      if (e.shiftKey && (active === firstEl || active === root)) {
        e.preventDefault();
        lastEl.focus();
      } else if (!e.shiftKey && active === lastEl) {
        e.preventDefault();
        firstEl.focus();
      }
    };
    root?.addEventListener("keydown", onKey);
    return () => {
      clearTimeout(t);
      root?.removeEventListener("keydown", onKey);
      prev?.focus?.();
    };
  }, [open, panel, first]);
}

const FOCUSABLE = [
  "a[href]",
  "button:not(:disabled)",
  "input:not(:disabled):not([type=hidden])",
  "select:not(:disabled)",
  "textarea:not(:disabled)",
  "[tabindex]:not([tabindex='-1'])",
].join(", ");

/** Where a dialog puts focus when it opens: the element marked `data-autofocus`, else the first field, else the panel itself. */
function dialogFocusTarget(panel: HTMLElement): HTMLElement | null {
  return (
    panel.querySelector<HTMLElement>("[data-autofocus]:not(:disabled)") ??
    panel.querySelector<HTMLElement>("textarea:not(:disabled), input:not([type=hidden]):not(:disabled), select:not(:disabled)")
  );
}

/** Centered modal. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  wide,
  description,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
  description?: ReactNode;
}) {
  useEscape(open, onClose);
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();
  // Focus the marked element or the first field for keyboard users.
  useModalFocus(open, panel, dialogFocusTarget);
  if (!open) return null;
  return createPortal(
    <div className="pinhole-fade fixed inset-0 z-50 flex items-center justify-center bg-neutral-950/50 p-4 backdrop-blur-[2px]" onMouseDown={onClose}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className={cx(
          "pinhole-pop flex max-h-[90vh] w-full flex-col overflow-hidden rounded-2xl border border-neutral-200 bg-white shadow-2xl outline-none dark:border-neutral-800 dark:bg-neutral-900",
          wide ? "max-w-3xl" : "max-w-lg",
        )}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 px-5 pt-4 pb-3">
          <div className="min-w-0">
            <div id={titleId} className="text-base font-semibold">
              {title}
            </div>
            {description && <div className="mt-0.5 text-sm text-neutral-500">{description}</div>}
          </div>
          <IconButton label="Close" size="sm" onClick={onClose} className="-mr-1.5">
            <X className="h-4 w-4" />
          </IconButton>
        </div>
        <div className="min-h-0 overflow-auto px-5 pb-4">{children}</div>
        {footer && (
          <div className="flex flex-wrap items-center justify-end gap-2 border-t border-neutral-200 bg-neutral-50 px-5 py-3 dark:border-neutral-800 dark:bg-neutral-900/60">
            {footer}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

/** Right-side panel (Settings, Fine-tune on narrow screens). */
export function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title: ReactNode; children: ReactNode }) {
  useEscape(open, onClose);
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();
  // Focus the panel itself (not the first field), so Settings doesn't jump into an input.
  useModalFocus(open, panel);
  if (!open) return null;
  return createPortal(
    <div className="pinhole-fade fixed inset-0 z-40 flex justify-end bg-neutral-950/40" onMouseDown={onClose}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="pinhole-slide h-full w-full max-w-md overflow-auto border-l border-neutral-200 bg-white shadow-2xl outline-none dark:border-neutral-800 dark:bg-neutral-900"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="sticky top-0 z-10 flex items-center justify-between border-b border-neutral-200 bg-white/95 px-5 py-3 backdrop-blur dark:border-neutral-800 dark:bg-neutral-900/95">
          <span id={titleId} className="text-base font-semibold">
            {title}
          </span>
          <IconButton label="Close" size="sm" onClick={onClose}>
            <X className="h-4 w-4" />
          </IconButton>
        </div>
        <div className="px-5 py-4">{children}</div>
      </div>
    </div>,
    document.body,
  );
}

/** Plain-language error with the technical output behind a "Details" toggle. */
export function ErrorNotice({ error, onDismiss, action }: { error: CoreError; onDismiss?: () => void; action?: ReactNode }) {
  const [show, setShow] = useState(false);
  return (
    <div role="alert" className="rounded-lg border border-red-200 bg-red-50 p-3 text-sm text-red-900 dark:border-red-900/70 dark:bg-red-950/40 dark:text-red-200">
      <div className="flex items-start gap-2.5">
        <CircleAlert className="mt-0.5 h-4 w-4 shrink-0" aria-hidden />
        <div className="min-w-0 flex-1">
          <p>{error.message}</p>
          {(error.details || action) && (
            <div className="mt-1.5 flex flex-wrap items-center gap-3">
              {action}
              {error.details && (
                <button type="button" className={cx("rounded text-xs font-medium underline underline-offset-2", focusRing)} onClick={() => setShow((s) => !s)}>
                  {show ? "Hide details" : "Details"}
                </button>
              )}
            </div>
          )}
          {show && error.details && (
            <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap rounded-md bg-black/5 p-2 font-mono text-xs dark:bg-white/5">{error.details}</pre>
          )}
        </div>
        {onDismiss && (
          <IconButton label="Dismiss" size="sm" onClick={onDismiss} className="-my-1 -mr-1 text-red-700 hover:bg-red-100 dark:text-red-300 dark:hover:bg-red-900/40">
            <X className="h-4 w-4" />
          </IconButton>
        )}
      </div>
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
  "w-full rounded-lg border border-neutral-300 bg-white px-3 py-2 text-sm text-neutral-900 placeholder:text-neutral-400 outline-none transition-colors focus:border-amber-500 focus:ring-2 focus:ring-amber-500/30 disabled:opacity-60 dark:border-neutral-700 dark:bg-neutral-950 dark:text-neutral-100 dark:placeholder:text-neutral-500";

export const selectClass = `${inputClass} appearance-none pr-8 bg-no-repeat`;

/** Native <select> with a chevron. */
export function Select({
  value,
  onChange,
  children,
  ariaLabel,
  className = "",
  disabled,
}: {
  value: string;
  onChange: (v: string) => void;
  children: ReactNode;
  ariaLabel?: string;
  className?: string;
  disabled?: boolean;
}) {
  return (
    <div className={cx("relative", className)}>
      <select aria-label={ariaLabel} disabled={disabled} className={cx(selectClass, "h-9 py-0")} value={value} onChange={(e) => onChange(e.target.value)}>
        {children}
      </select>
      <ChevronDown aria-hidden className="pointer-events-none absolute top-1/2 right-2.5 h-4 w-4 -translate-y-1/2 text-neutral-400" />
    </div>
  );
}

/**
 * Textarea that grows with its content (up to maxRows).
 * PRIVACY: spellcheck/autocomplete are off by default — some WebView spellcheckers
 * send text to cloud services, and autofill can remember what was typed.
 */
export function AutoTextarea({
  minRows = 3,
  maxRows = 12,
  className = "",
  value,
  ref: outerRef,
  bare = false,
  ...rest
}: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  minRows?: number;
  maxRows?: number;
  value: string;
  ref?: Ref<HTMLTextAreaElement>;
  /** No border/background of its own (inside a styled container). */
  bare?: boolean;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const setRef = useCallback(
    (el: HTMLTextAreaElement | null) => {
      ref.current = el;
      if (typeof outerRef === "function") outerRef(el);
      else if (outerRef) (outerRef as { current: HTMLTextAreaElement | null }).current = el;
    },
    [outerRef],
  );
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const cs = getComputedStyle(el);
    const lh = parseFloat(cs.lineHeight) || 20;
    const padY = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0);
    const border = (parseFloat(cs.borderTopWidth) || 0) + (parseFloat(cs.borderBottomWidth) || 0);
    el.style.height = "auto";
    const min = lh * minRows + padY + border;
    const max = lh * maxRows + padY + border;
    const want = el.scrollHeight + border;
    el.style.height = `${Math.ceil(Math.min(Math.max(want, min), max))}px`;
    el.style.overflowY = want > max ? "auto" : "hidden";
  }, [value, minRows, maxRows]);
  return (
    <textarea
      ref={setRef}
      rows={minRows}
      spellCheck={false}
      autoComplete="off"
      autoCorrect="off"
      autoCapitalize="off"
      className={cx(
        bare
          ? "block w-full bg-transparent px-3 py-2 text-sm text-neutral-900 outline-none placeholder:text-neutral-400 dark:text-neutral-100 dark:placeholder:text-neutral-500"
          : inputClass,
        "resize-none leading-relaxed",
        className,
      )}
      value={value}
      {...rest}
    />
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="rounded border border-current/25 px-1 py-px font-sans text-[10px] font-medium leading-none opacity-80">{children}</kbd>
  );
}

/** Small uppercase label used for control rows. */
export function Label({ children, htmlFor, className = "" }: { children: ReactNode; htmlFor?: string; className?: string }) {
  return (
    <label htmlFor={htmlFor} className={cx("text-xs font-medium text-neutral-500 dark:text-neutral-400", className)}>
      {children}
    </label>
  );
}

// ------------------------------------------------------------------ Popover / Menu

/**
 * Anchored popover rendered in a portal with fixed positioning (never clipped
 * by scroll containers). Closes on outside click, Escape, resize.
 */
export function Popover({
  trigger,
  children,
  align = "start",
  width,
  className = "",
  open: controlledOpen,
  onOpenChange,
}: {
  trigger: (p: { onClick: () => void; "aria-expanded": boolean; "aria-haspopup": "dialog" | "menu" | "listbox"; ref: (el: HTMLElement | null) => void }) => ReactNode;
  children: ReactNode | ((close: () => void) => ReactNode);
  align?: "start" | "end";
  width?: number | "trigger";
  className?: string;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}) {
  const [innerOpen, setInnerOpen] = useState(false);
  const open = controlledOpen ?? innerOpen;
  const setOpen = useCallback(
    (v: boolean) => {
      setInnerOpen(v);
      onOpenChange?.(v);
    },
    [onOpenChange],
  );
  const anchor = useRef<HTMLElement | null>(null);
  const panel = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<CSSProperties>({});
  const close = useCallback(() => setOpen(false), [setOpen]);

  useLayoutEffect(() => {
    if (!open || !anchor.current) return;
    const place = () => {
      const r = anchor.current!.getBoundingClientRect();
      const w = width === "trigger" ? r.width : width;
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const s: CSSProperties = { position: "fixed", zIndex: 60, maxHeight: Math.max(160, vh - r.bottom - 16) };
      if (w) s.width = Math.min(w, vw - 16);
      if (align === "end") s.right = Math.max(8, vw - r.right);
      else s.left = Math.min(Math.max(8, r.left), Math.max(8, vw - (typeof w === "number" ? w : 240) - 8));
      // Flip above when there isn't room below.
      if (vh - r.bottom < 220 && r.top > vh - r.bottom) {
        s.bottom = vh - r.top + 6;
        s.maxHeight = r.top - 16;
      } else s.top = r.bottom + 6;
      setStyle(s);
    };
    place();
    // Follow the trigger when a scroll container (e.g. the sidebar) scrolls; the
    // capture phase catches scrolls of any ancestor. The menu's own scroll is ignored.
    const onScroll = (e: Event) => {
      if (e.target instanceof Node && panel.current?.contains(e.target)) return;
      place();
    };
    window.addEventListener("resize", place);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open, width, align]);

  useEscape(open, close);
  // Keyboard: move focus into the menu on open (the selected item, else the first
  // control), and back to the trigger on close, unless the user clicked elsewhere.
  const closedByOutsideClick = useRef(false);
  useEffect(() => {
    if (!open) return;
    closedByOutsideClick.current = false;
    const el = panel.current;
    const t = setTimeout(() => {
      const target =
        el?.querySelector<HTMLElement>('[aria-current="true"]:not(:disabled)') ??
        el?.querySelector<HTMLElement>("input:not([type=hidden]), textarea, [role=menuitem]:not(:disabled), button:not(:disabled)");
      (target ?? el)?.focus();
    }, 0);
    return () => {
      clearTimeout(t);
      // A click on a part of the page that can't take focus leaves it on body: don't
      // pull focus (and the sidebar's scroll) back to the trigger then.
      if (closedByOutsideClick.current) return;
      const active = document.activeElement;
      if (!active || active === document.body || el?.contains(active)) anchor.current?.focus();
    };
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (panel.current?.contains(t) || anchor.current?.contains(t)) return;
      closedByOutsideClick.current = true;
      close();
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open, close]);

  return (
    <>
      {trigger({
        onClick: () => setOpen(!open),
        "aria-expanded": open,
        "aria-haspopup": "dialog",
        ref: (el) => {
          anchor.current = el;
        },
      })}
      {open &&
        createPortal(
          <div
            ref={panel}
            style={style}
            tabIndex={-1}
            onKeyDown={menuArrowKeys}
            className={cx(
              "pinhole-pop overflow-auto outline-none rounded-xl border border-neutral-200 bg-white p-1 shadow-xl dark:border-neutral-700 dark:bg-neutral-900",
              className,
            )}
          >
            {typeof children === "function" ? children(close) : children}
          </div>,
          document.body,
        )}
    </>
  );
}

/** Up/Down (Home/End) move between a popover's menu items, wrapping around. Fields keep their own keys. */
function menuArrowKeys(e: ReactKeyboardEvent<HTMLElement>) {
  const from = e.target as HTMLElement;
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key) || e.altKey || e.ctrlKey || e.metaKey) return;
  if (from !== e.currentTarget && from.getAttribute("role") !== "menuitem") return;
  const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[role=menuitem]:not(:disabled)")];
  if (!items.length) return;
  e.preventDefault();
  const i = items.indexOf(from);
  const next =
    e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : i < 0 ? (e.key === "ArrowDown" ? 0 : items.length - 1) : (i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
  items[next].focus();
}

export function MenuItem({
  children,
  onClick,
  selected,
  disabled,
  danger,
  hint,
  icon,
  right,
}: {
  children: ReactNode;
  onClick?: () => void;
  selected?: boolean;
  disabled?: boolean;
  danger?: boolean;
  hint?: ReactNode;
  icon?: ReactNode;
  right?: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      disabled={disabled}
      onClick={onClick}
      aria-current={selected || undefined}
      className={cx(
        "flex w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        focusRing,
        selected ? "bg-amber-50 dark:bg-amber-500/10" : "enabled:hover:bg-neutral-100 dark:enabled:hover:bg-neutral-800",
        danger && "text-red-700 dark:text-red-400",
      )}
    >
      {icon && <span className="mt-0.5 shrink-0 text-neutral-500">{icon}</span>}
      <span className="min-w-0 flex-1">
        <span className="block truncate">{children}</span>
        {hint && <span className="mt-0.5 block text-xs text-neutral-500">{hint}</span>}
      </span>
      {right && <span className="shrink-0">{right}</span>}
    </button>
  );
}

export function MenuSeparator() {
  return <div role="separator" className="my-1 h-px bg-neutral-200 dark:bg-neutral-800" />;
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <div className="px-2.5 pt-2 pb-1 text-[11px] font-semibold tracking-wide text-neutral-400 uppercase">{children}</div>;
}
