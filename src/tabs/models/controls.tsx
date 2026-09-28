// Small building blocks used by the Models tab, Settings and First run (frontend B).
// The shared primitives live in src/components/ui (frontend A) and are only imported here.
import { useId, useState, type ReactNode } from "react";
import { Check, Eye, EyeOff, KeyRound, ShieldCheck } from "lucide-react";
import { asCoreError, setCivitaiKey } from "../../lib/api";
import type { CoreError, FamilyChoice, GroupStatus } from "../../lib/types";
import { Button, Dialog, ErrorNotice, ProgressBar, Select as UiSelect, Spinner, focusRing, inputClass } from "../../components/ui";
import { groupFraction, isActive, progressText, stateLabel } from "./lib/words";

/** Typed wrapper around the shared native Select. */
export function Select<T extends string | number>({
  value,
  onChange,
  options,
  label,
  className = "",
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  label: string;
  className?: string;
}) {
  return (
    <UiSelect
      ariaLabel={label}
      className={className}
      value={String(value)}
      onChange={(v) => {
        const found = options.find((o) => String(o.value) === v);
        if (found) onChange(found.value);
      }}
    >
      {options.map((o) => (
        <option key={String(o.value)} value={String(o.value)}>
          {o.label}
        </option>
      ))}
    </UiSelect>
  );
}

export function Chip({ active, onClick, children, title }: { active: boolean; onClick: () => void; children: ReactNode; title?: string }) {
  return (
    <button
      type="button"
      aria-pressed={active}
      title={title}
      onClick={onClick}
      className={`inline-flex h-7 items-center gap-1 rounded-full border px-3 text-xs font-medium transition-colors ${focusRing} ${
        active
          ? "border-amber-500 bg-amber-500 text-neutral-950"
          : "border-neutral-300 bg-white text-neutral-700 hover:border-neutral-400 hover:text-neutral-900 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:border-neutral-500 dark:hover:text-white"
      }`}
    >
      {children}
    </button>
  );
}

/** Label + control stacked, for the filter bar. */
export function FilterGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-xs font-medium text-neutral-500 dark:text-neutral-400">{label}</span>
      {children}
    </div>
  );
}

export function EmptyState({ icon, title, children, actions }: { icon?: ReactNode; title: ReactNode; children?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="mx-auto flex max-w-md flex-col items-center gap-3 px-6 py-14 text-center">
      {icon && <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-neutral-200/70 text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">{icon}</div>}
      <h3 className="text-base font-semibold text-neutral-900 dark:text-neutral-100">{title}</h3>
      {children && <div className="text-sm text-neutral-600 dark:text-neutral-400">{children}</div>}
      {actions && <div className="mt-1 flex flex-wrap justify-center gap-2">{actions}</div>}
    </div>
  );
}

export function Skeleton({ className = "" }: { className?: string }) {
  return <div className={`animate-pulse rounded-md bg-neutral-200 dark:bg-neutral-800 ${className}`} />;
}

/** Progress of one download group with an optional Cancel. */
export function GroupProgress({ group, onCancel, compact }: { group: GroupStatus; onCancel?: () => void; compact?: boolean }) {
  const active = isActive(group);
  return (
    <div className="space-y-1.5" aria-live="polite">
      <div className="flex items-center justify-between gap-2 text-xs text-neutral-600 dark:text-neutral-400">
        <span className="flex min-w-0 items-center gap-1.5">
          {active && <Spinner className="h-3 w-3 text-amber-500" />}
          <span className="truncate">
            {group.state === "downloading" || group.state === "queued" ? progressText(group) : stateLabel(group.state)}
          </span>
        </span>
        {active && onCancel && (
          <button type="button" onClick={onCancel} className="shrink-0 rounded px-1 font-medium text-neutral-600 hover:text-red-600 hover:underline dark:text-neutral-400 dark:hover:text-red-400">
            Cancel
          </button>
        )}
      </div>
      <ProgressBar value={groupFraction(group)} indeterminate={group.state === "verifying" || group.state === "queued"} />
      {!compact && active && group.fileCount > 1 && (
        <div className="truncate text-[11px] text-neutral-500">
          File {Math.min(group.fileIndex + 1, group.fileCount)} of {group.fileCount}
          {group.currentFile ? ` · ${group.currentFile}` : ""}
        </div>
      )}
    </div>
  );
}

/** "Which kind of model is this?" radio list (ambiguous detection, SPEC §6 step 4). */
export function FamilyPicker({ candidates, value, onChange }: { candidates: FamilyChoice[]; value: string | null; onChange: (id: string) => void }) {
  const name = useId();
  return (
    <fieldset className="space-y-1.5">
      {candidates.map((c) => (
        <label
          key={c.familyId}
          className={`flex cursor-pointer items-center gap-3 rounded-lg border px-3 py-2 text-sm transition-colors ${
            value === c.familyId
              ? "border-amber-500 bg-amber-50 dark:bg-amber-500/10"
              : "border-neutral-200 hover:border-neutral-300 dark:border-neutral-800 dark:hover:border-neutral-700"
          }`}
        >
          <input type="radio" name={name} className="accent-amber-500" checked={value === c.familyId} onChange={() => onChange(c.familyId)} />
          <span>{c.label}</span>
        </label>
      ))}
    </fieldset>
  );
}

/** "Show 18+ content?" — once per session (the answer lives in RAM only). */
export function AdultConfirmDialog({ open, onCancel, onConfirm }: { open: boolean; onCancel: () => void; onConfirm: () => void }) {
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title="Show 18+ content?"
      footer={
        <>
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" onClick={onConfirm}>
            I'm 18 or older
          </Button>
        </>
      }
    >
      <div className="space-y-2 text-sm text-neutral-700 dark:text-neutral-300">
        <p>Some models and preview images on CivitAI are made for adults. Confirm you're 18 or older to include them.</p>
        <p className="text-neutral-500">Pinhole asks once and forgets your answer when you close the app.</p>
      </div>
    </Dialog>
  );
}

/** Explains why a CivitAI key helps, saves it to the OS keychain. The key is only kept in this input until saved. */
export function ApiKeyDialog({ open, onClose, onSaved, reason }: { open: boolean; onClose: () => void; onSaved: () => void; reason?: string | null }) {
  const [key, setKey] = useState("");
  const [show, setShow] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const inputId = useId();

  const close = () => {
    setKey("");
    setError(null);
    setShow(false);
    onClose();
  };

  const save = async () => {
    const k = key.trim();
    if (!k) return;
    setSaving(true);
    setError(null);
    try {
      await setCivitaiKey(k);
      setKey("");
      setShow(false);
      onSaved();
    } catch (e) {
      setError(asCoreError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open={open}
      onClose={close}
      title={
        <span className="flex items-center gap-2">
          <KeyRound className="h-4 w-4 text-amber-500" /> Add your CivitAI API key
        </span>
      }
      footer={
        <>
          <Button variant="ghost" onClick={close}>
            Not now
          </Button>
          <Button variant="primary" onClick={save} disabled={!key.trim() || saving}>
            {saving ? <Spinner className="h-3.5 w-3.5" /> : <Check className="h-4 w-4" />} Save key
          </Button>
        </>
      }
    >
      <form
        className="space-y-3 text-sm text-neutral-700 dark:text-neutral-300"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        {reason && <p className="font-medium text-neutral-900 dark:text-neutral-100">{reason}</p>}
        <p>
          Some CivitAI models only download for signed-in users. Create a key on civitai.com under <b>Account settings → API keys</b> and paste it here. It's
          optional — most models download without one.
        </p>
        <div className="space-y-1">
          <label htmlFor={inputId} className="text-xs font-medium text-neutral-600 dark:text-neutral-400">
            API key
          </label>
          <div className="relative">
            <input
              id={inputId}
              type={show ? "text" : "password"}
              autoComplete="off"
              spellCheck={false}
              autoFocus
              value={key}
              onChange={(e) => setKey(e.target.value)}
              className={`${inputClass} pr-10 font-mono`}
              placeholder="Paste your key"
            />
            <button
              type="button"
              onClick={() => setShow((s) => !s)}
              className="absolute top-1/2 right-2 -translate-y-1/2 rounded p-1 text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
              aria-label={show ? "Hide key" : "Show key"}
            >
              {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
            </button>
          </div>
        </div>
        <p className="flex items-start gap-2 rounded-lg bg-neutral-100 p-2.5 text-xs text-neutral-600 dark:bg-neutral-800/60 dark:text-neutral-400">
          <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
          Stored in your system's keychain, never in Pinhole's Data folder. It is only sent to civitai.com when you download.
        </p>
        {error && <ErrorNotice error={error} />}
      </form>
    </Dialog>
  );
}
