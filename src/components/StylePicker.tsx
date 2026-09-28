// Style picker (None + saved styles), "＋ Save as style" and the Manage dialog.
// Styles are the ONE thing typed text may be stored for — and only when the
// user explicitly saves one (SPEC §4.11). The UI says so where it happens.
import { useState } from "react";
import { Check, ChevronDown, Copy, Lock, Palette, Pencil, Plus, Settings2, Trash } from "lucide-react";
import * as api from "../lib/api";
import type { CoreError, Style } from "../lib/types";
import { useActions } from "../lib/state/AppProvider";
import { useAppState } from "../lib/state/store";
import { AutoTextarea, Button, Dialog, ErrorNotice, Field, IconButton, MenuItem, MenuLabel, MenuSeparator, Popover, Toggle, cx, focusRing, inputClass } from "./ui";

export const STORED_NOTE = "Saved styles are stored on this computer.";

function styleFits(st: Style, familyId: string | null | undefined) {
  return !st.families.length || !familyId || st.families.includes(familyId);
}

export function StylePicker({
  value,
  onChange,
  familyId,
  familyLabel,
  compact = false,
}: {
  value: string | null;
  onChange: (id: string | null) => void;
  familyId?: string | null;
  familyLabel?: string | null;
  compact?: boolean;
}) {
  const styles = useAppState((s) => s.styles);
  const current = styles.find((s) => s.id === value) ?? null;
  const [saveOpen, setSaveOpen] = useState(false);
  const [manageOpen, setManageOpen] = useState(false);
  const builtins = styles.filter((s) => s.builtin);
  const mine = styles.filter((s) => !s.builtin);

  const item = (st: Style, close: () => void) => (
    <MenuItem
      key={st.id}
      selected={st.id === value}
      onClick={() => {
        onChange(st.id);
        close();
      }}
      right={st.id === value ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
      hint={
        <span className="line-clamp-2">
          {st.positive}
          {!styleFits(st, familyId) && <span className="text-amber-700 dark:text-amber-400"> · written for other models</span>}
        </span>
      }
    >
      {st.name}
    </MenuItem>
  );

  return (
    <div className={cx("flex min-w-0 items-center gap-2", compact && "flex-wrap")}>
      <Popover
        width={340}
        trigger={(p) => (
          <button
            {...p}
            type="button"
            aria-label={`Style: ${current?.name ?? "None"}`}
            className={cx(
              "inline-flex h-8 min-w-0 items-center gap-1.5 rounded-lg border border-neutral-200 bg-white px-2.5 text-sm shadow-xs hover:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:hover:border-neutral-600",
              focusRing,
            )}
          >
            <Palette className="h-3.5 w-3.5 shrink-0 text-neutral-500" />
            <span className="text-neutral-500">Style</span>
            <span className={cx("truncate font-medium", current ? "text-neutral-900 dark:text-white" : "text-neutral-600 dark:text-neutral-300")}>
              {current?.name ?? "None"}
            </span>
            <ChevronDown className="h-3.5 w-3.5 shrink-0 text-neutral-400" />
          </button>
        )}
      >
        {(close) => (
          <div role="listbox" aria-label="Style">
            <MenuItem
              selected={!value}
              onClick={() => {
                onChange(null);
                close();
              }}
              right={!value ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
              hint="Just your prompt"
            >
              None
            </MenuItem>
            {mine.length > 0 && <MenuLabel>Your styles</MenuLabel>}
            {mine.map((st) => item(st, close))}
            {builtins.length > 0 && <MenuLabel>Built in</MenuLabel>}
            {builtins.map((st) => item(st, close))}
            <MenuSeparator />
            <MenuItem
              icon={<Settings2 className="h-4 w-4" />}
              onClick={() => {
                close();
                setManageOpen(true);
              }}
            >
              Manage styles…
            </MenuItem>
          </div>
        )}
      </Popover>
      <button
        type="button"
        onClick={() => setSaveOpen(true)}
        className={cx("inline-flex shrink-0 items-center gap-1 rounded-md px-1.5 py-1 text-xs font-medium text-amber-700 hover:bg-amber-50 dark:text-amber-400 dark:hover:bg-amber-500/10", focusRing)}
      >
        <Plus className="h-3.5 w-3.5" /> Save as style
      </button>

      <StyleEditorDialog
        open={saveOpen}
        onClose={() => setSaveOpen(false)}
        initial={current ? { ...current, id: "", name: current.builtin ? current.name : `${current.name} (copy)`, builtin: false } : null}
        familyId={familyId ?? null}
        familyLabel={familyLabel ?? null}
        onSaved={(st) => onChange(st.id)}
      />
      <ManageStylesDialog open={manageOpen} onClose={() => setManageOpen(false)} familyId={familyId ?? null} familyLabel={familyLabel ?? null} />
    </div>
  );
}

/** Create or edit a user style. `initial.id === ""` creates a new one. */
export function StyleEditorDialog({
  open,
  onClose,
  initial,
  familyId,
  familyLabel,
  onSaved,
}: {
  open: boolean;
  onClose: () => void;
  initial: Style | null;
  familyId: string | null;
  familyLabel: string | null;
  onSaved?: (s: Style) => void;
}) {
  return open ? (
    <StyleEditorInner onClose={onClose} initial={initial} familyId={familyId} familyLabel={familyLabel} onSaved={onSaved} />
  ) : null;
}

function StyleEditorInner({
  onClose,
  initial,
  familyId,
  familyLabel,
  onSaved,
}: {
  onClose: () => void;
  initial: Style | null;
  familyId: string | null;
  familyLabel: string | null;
  onSaved?: (s: Style) => void;
}) {
  const actions = useActions();
  const editing = !!initial?.id;
  const [name, setName] = useState(initial?.name ?? "");
  const [positive, setPositive] = useState(initial?.positive ?? "");
  const [negative, setNegative] = useState(initial?.negative ?? "");
  const initialFamilies = initial?.families ?? [];
  const [onlyFamily, setOnlyFamily] = useState(initialFamilies.length > 0);
  const limitedFamilies = initialFamilies.length ? initialFamilies : familyId ? [familyId] : [];
  const limitLabel = initialFamilies.length ? "Only for the kinds of models it's written for" : `Only for ${familyLabel ?? "this kind of"} models`;
  const [error, setError] = useState<CoreError | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    if (!name.trim() || !positive.trim()) return;
    setSaving(true);
    setError(null);
    try {
      const families = onlyFamily ? limitedFamilies : [];
      const saved = await api.saveStyle({
        id: initial?.id ?? "",
        name: name.trim(),
        positive: positive.trim(),
        negative: negative.trim() || null,
        families,
        thumbnail: initial?.thumbnail ?? null,
        builtin: false,
      });
      await actions.refreshStyles();
      onSaved?.(saved);
      actions.toast(editing ? `Style “${saved.name}” updated` : `Saved style “${saved.name}”`);
      onClose();
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title={editing ? "Edit style" : "Save as style"}
      description="A style describes how things look. It's added to any prompt when you pick it."
      footer={
        <>
          <span className="mr-auto inline-flex items-center gap-1.5 text-xs text-neutral-500">
            <Lock className="h-3.5 w-3.5" /> {STORED_NOTE}
          </span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!name.trim() || !positive.trim() || saving} onClick={() => void save()}>
            {editing ? "Save changes" : "Save style"}
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <Field label="Name">
          <input
            data-autofocus
            className={inputClass}
            value={name}
            maxLength={60}
            spellCheck={false}
            autoComplete="off"
            placeholder="e.g. Soft film look"
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Looks like" hint="e.g. 35mm film photo, soft window light, shallow depth of field">
          <AutoTextarea minRows={3} value={positive} onChange={(e) => setPositive(e.target.value)} placeholder="Words that describe the look" />
        </Field>
        <Field label="Avoid (optional)" hint="Only used by models that support a negative prompt.">
          <AutoTextarea minRows={2} value={negative} onChange={(e) => setNegative(e.target.value)} placeholder="e.g. cartoon, plastic skin" />
        </Field>
        {limitedFamilies.length > 0 && <Toggle checked={onlyFamily} onChange={setOnlyFamily} label={limitLabel} />}
        {error && <ErrorNotice error={error} />}
      </div>
    </Dialog>
  );
}

function ManageStylesDialog({ open, onClose, familyId, familyLabel }: { open: boolean; onClose: () => void; familyId: string | null; familyLabel: string | null }) {
  const styles = useAppState((s) => s.styles);
  const actions = useActions();
  const [editing, setEditing] = useState<Style | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [error, setError] = useState<CoreError | null>(null);

  const duplicate = async (st: Style) => {
    try {
      const saved = await api.saveStyle({ ...st, id: "", name: `${st.name} (copy)`, builtin: false });
      await actions.refreshStyles();
      actions.toast(`Duplicated as “${saved.name}”`);
    } catch (e) {
      setError(api.asCoreError(e));
    }
  };
  const remove = async (st: Style) => {
    try {
      await api.deleteStyle(st.id);
      await actions.refreshStyles();
      setConfirmDelete(null);
    } catch (e) {
      setError(api.asCoreError(e));
    }
  };

  return (
    <>
      <Dialog
        open={open && !editing}
        onClose={onClose}
        title="Styles"
        description={STORED_NOTE}
        wide
        footer={
          <Button variant="primary" onClick={() => setEditing({ id: "", name: "", positive: "", negative: null, families: [], thumbnail: null, builtin: false })}>
            <Plus className="h-4 w-4" /> New style
          </Button>
        }
      >
        {error && (
          <div className="mb-3">
            <ErrorNotice error={error} onDismiss={() => setError(null)} />
          </div>
        )}
        <ul className="divide-y divide-neutral-200 dark:divide-neutral-800">
          {styles.map((st) => (
            <li key={st.id} className="flex items-start gap-3 py-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-sm font-medium">
                  {st.name}
                  {st.builtin && (
                    <span className="inline-flex items-center gap-1 text-xs font-normal text-neutral-500">
                      <Lock className="h-3 w-3" /> Built in
                    </span>
                  )}
                </div>
                <p className="mt-0.5 line-clamp-2 text-xs text-neutral-500">{st.positive}</p>
                {st.negative && <p className="mt-0.5 line-clamp-1 text-xs text-neutral-400">Avoid: {st.negative}</p>}
              </div>
              {confirmDelete === st.id ? (
                <div className="flex shrink-0 items-center gap-1.5">
                  <span className="text-xs text-neutral-500">Delete?</span>
                  <Button size="sm" variant="danger" onClick={() => void remove(st)}>
                    Delete
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setConfirmDelete(null)}>
                    Keep
                  </Button>
                </div>
              ) : (
                <div className="flex shrink-0 items-center gap-0.5">
                  {!st.builtin && (
                    <IconButton label={`Edit ${st.name}`} size="sm" onClick={() => setEditing(st)}>
                      <Pencil className="h-3.5 w-3.5" />
                    </IconButton>
                  )}
                  <IconButton label={`Duplicate ${st.name}`} size="sm" onClick={() => void duplicate(st)}>
                    <Copy className="h-3.5 w-3.5" />
                  </IconButton>
                  {!st.builtin && (
                    <IconButton label={`Delete ${st.name}`} size="sm" onClick={() => setConfirmDelete(st.id)}>
                      <Trash className="h-3.5 w-3.5" />
                    </IconButton>
                  )}
                </div>
              )}
            </li>
          ))}
          {!styles.length && <li className="py-6 text-center text-sm text-neutral-500">No styles yet.</li>}
        </ul>
      </Dialog>
      <StyleEditorDialog open={!!editing} onClose={() => setEditing(null)} initial={editing} familyId={familyId} familyLabel={familyLabel} />
    </>
  );
}
