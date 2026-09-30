// Preset picker next to the model picker (SPEC §7). Presets never contain the prompt.
import { useRef, useState } from "react";
import { Bookmark, BookmarkPlus, Check, ChevronDown, Lock, Trash } from "lucide-react";
import { Button, Dialog, ErrorNotice, Field, IconButton, MenuItem, MenuLabel, MenuSeparator, Popover, cx, focusRing, inputClass } from "../../components/ui";
import * as api from "../../lib/api";
import { SHAPE_LABEL } from "../../lib/paste/map";
import type { CoreError, Preset } from "../../lib/types";
import { useActions } from "../../lib/state/AppProvider";
import { applyPreset, clearPreset, presetFromCreate, type PresetApplication } from "../../lib/state/request";
import { useAppState, useStore } from "../../lib/state/store";

/** "balanced" → "Balanced", as the Quality dial shows it. */
const qualityLabel = (q: string) => q[0].toUpperCase() + q.slice(1);

export interface PresetNotice {
  preset: Preset;
  app: PresetApplication;
}

export function PresetPicker({ onApplied }: { onApplied: (n: PresetNotice | null) => void }) {
  const presets = useAppState((s) => s.presets);
  const presetId = useAppState((s) => s.create.presetId);
  const store = useStore();
  const actions = useActions();
  const [saveOpen, setSaveOpen] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);
  const current = presets.find((p) => p.id === presetId) ?? null;
  const builtin = presets.filter((p) => p.builtin);
  const mine = presets.filter((p) => !p.builtin);

  const pick = (p: Preset) => {
    const s = store.getState();
    const app = applyPreset(p, s.create, { models: s.models ?? [], loras: s.loras, styleIds: s.styles.map((x) => x.id) });
    store.dispatch({ type: "patchCreate", patch: app.patch });
    onApplied(app.missingModel || app.missingLoras.length || app.missingStyle ? { preset: p, app } : null);
  };

  const pickNone = () => {
    const s = store.getState();
    store.dispatch({ type: "patchCreate", patch: clearPreset(s.create, { models: s.models ?? [], loras: s.loras, styleIds: s.styles.map((x) => x.id) }) });
    onApplied(null);
  };

  const remove = async (p: Preset) => {
    try {
      await api.deletePreset(p.id);
      await actions.refreshPresets();
      setConfirm(null);
    } catch (e) {
      actions.toast(api.asCoreError(e).message, { tone: "error" });
    }
  };

  const item = (p: Preset, close: () => void) => (
    <div key={p.id} className="group flex items-center gap-1">
      <div className="min-w-0 flex-1">
        <MenuItem
          selected={p.id === presetId}
          onClick={() => {
            pick(p);
            close();
          }}
          right={p.id === presetId ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
          hint={[p.shape && SHAPE_LABEL[p.shape], p.quality && qualityLabel(p.quality), p.count && `${p.count} at a time`].filter(Boolean).join(" · ") || undefined}
        >
          {p.name}
        </MenuItem>
      </div>
      {!p.builtin &&
        (confirm === p.id ? (
          <Button size="sm" variant="danger" onClick={() => void remove(p)}>
            Delete
          </Button>
        ) : (
          <IconButton label={`Delete preset ${p.name}`} size="sm" className="opacity-0 group-hover:opacity-100 focus-visible:opacity-100" onClick={() => setConfirm(p.id)}>
            <Trash className="h-3.5 w-3.5" />
          </IconButton>
        ))}
    </div>
  );

  return (
    <>
      <Popover
        align="end"
        width={300}
        onOpenChange={(o) => !o && setConfirm(null)}
        trigger={(p) => (
          <button
            {...p}
            type="button"
            aria-label={`Preset: ${current?.name ?? "none"}`}
            className={cx(
              "flex w-32 shrink-0 flex-col justify-center self-stretch rounded-xl border border-neutral-200 bg-white px-3 py-2 text-left shadow-xs hover:border-neutral-300 dark:border-neutral-800 dark:bg-neutral-900 dark:hover:border-neutral-700",
              focusRing,
            )}
          >
            <span className="flex items-center gap-1 text-[11px] font-medium text-neutral-500">
              <Bookmark className="h-3 w-3" /> Preset
            </span>
            <span className="flex items-center gap-1">
              <span className={cx("truncate text-sm font-semibold", !current && "text-neutral-500 dark:text-neutral-400")}>{current?.name ?? "None"}</span>
              <ChevronDown className="ml-auto h-3.5 w-3.5 shrink-0 text-neutral-400" />
            </span>
          </button>
        )}
      >
        {(close) => (
          <div>
            <MenuItem
              selected={!presetId}
              onClick={() => {
                pickNone();
                close();
              }}
              right={!presetId ? <Check className="h-4 w-4 text-amber-600" /> : undefined}
              hint="Your own settings"
            >
              None
            </MenuItem>
            {mine.length > 0 && <MenuLabel>Your presets</MenuLabel>}
            {mine.map((p) => item(p, close))}
            {builtin.length > 0 && <MenuLabel>Built in</MenuLabel>}
            {builtin.map((p) => item(p, close))}
            {!presets.length && <div className="px-2.5 py-2 text-sm text-neutral-500">No presets yet.</div>}
            <MenuSeparator />
            <MenuItem
              icon={<BookmarkPlus className="h-4 w-4" />}
              onClick={() => {
                close();
                setSaveOpen(true);
              }}
              hint="Model, style, dials and Fine-tune — never your prompt"
            >
              Save as preset…
            </MenuItem>
          </div>
        )}
      </Popover>
      <SavePresetDialog open={saveOpen} onClose={() => setSaveOpen(false)} />
    </>
  );
}

export function SavePresetDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  return open ? <SavePresetInner onClose={onClose} /> : null;
}

function SavePresetInner({ onClose }: { onClose: () => void }) {
  const store = useStore();
  const actions = useActions();
  const [name, setName] = useState("");
  const [error, setError] = useState<CoreError | null>(null);
  const [saving, setSaving] = useState(false);
  // Enter in the name field submits the form even while the Save button is disabled;
  // a ref (not state) also stops two Enters in the same frame.
  const inflight = useRef(false);
  const s = store.getState();
  const model = (s.models ?? []).find((m) => m.id === s.create.modelId) ?? null;
  const style = s.styles.find((x) => x.id === s.create.styleId) ?? null;

  const save = async () => {
    if (!name.trim() || inflight.current) return;
    inflight.current = true;
    setSaving(true);
    setError(null);
    try {
      const st = store.getState();
      const preset = presetFromCreate(name, st.create, { model, loras: st.loras });
      const saved = await api.savePreset(preset);
      await actions.refreshPresets();
      store.dispatch({ type: "patchCreate", patch: { presetId: saved.id, presetBase: null } });
      actions.toast(`Saved preset “${saved.name}”`);
      onClose();
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      inflight.current = false;
      setSaving(false);
    }
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title="Save as preset"
      footer={
        <>
          <span className="mr-auto inline-flex items-center gap-1.5 text-xs text-neutral-500">
            <Lock className="h-3.5 w-3.5" /> Presets keep settings, not the prompt.
          </span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!name.trim() || saving} onClick={() => void save()}>
            Save preset
          </Button>
        </>
      }
    >
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Field label="Name">
          <input data-autofocus className={inputClass} value={name} maxLength={60} spellCheck={false} autoComplete="off" placeholder="e.g. Moody portraits" onChange={(e) => setName(e.target.value)} />
        </Field>
        <div className="rounded-lg bg-neutral-50 p-3 text-xs text-neutral-600 dark:bg-neutral-800/50 dark:text-neutral-400">
          <div className="mb-1 font-medium text-neutral-700 dark:text-neutral-300">Includes</div>
          <ul className="space-y-0.5">
            <li>Model: {model?.friendlyName ?? "—"}</li>
            <li>Style: {style?.name ?? "None"}</li>
            <li>
              Dials: {SHAPE_LABEL[s.create.shape]} · {qualityLabel(s.create.quality)} · {s.create.count} at a time
            </li>
            <li>Fine-tune: {Object.keys(s.create.fineTune).filter((k) => k !== "negativePrompt" && k !== "hiresScale" && k !== "hiresDenoise").length} changed setting(s)</li>
            <li>LoRAs: {s.create.loras.length || "none"}</li>
          </ul>
        </div>
        {error && <ErrorNotice error={error} />}
      </form>
    </Dialog>
  );
}
