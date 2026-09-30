// OWNER: frontend B. Settings sheet (SPEC §8). Every change is saved immediately.
// Keep this export signature.
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Check, CircleCheck, Download, FolderOpen, Info, KeyRound, ShieldCheck, TriangleAlert } from "lucide-react";
import {
  appInfo,
  asCoreError,
  clearCivitaiKey,
  getHardware,
  getSettings,
  hasCivitaiKey,
  onHardwareReady,
  openDataFolder,
  openOutputsFolder,
  setSettings as saveSettings,
} from "../lib/api";
import type { AppInfo, ContentMode, CoreError, HardwareView, Settings } from "../lib/types";
import { formatGb } from "../lib/format";
import { ShortcutsList } from "../components/ShortcutsList";
import { useHelperModels } from "../lib/helpers";
import { Badge, Button, ErrorNotice, Segmented, Sheet, Spinner, Toggle } from "../components/ui";
import { ApiKeyDialog, GroupProgress, Select, Skeleton } from "../tabs/models/controls";
import { cancelGroup } from "../tabs/models/lib/downloads";
import { useEngine, useTauriEvent } from "../tabs/models/lib/hooks";
import { backendShort, isCpuOnly, tierPlain } from "../tabs/models/lib/words";
import { emitSettingsChanged } from "./events";
import { EngineOutput } from "./EngineOutput";
import { ModelsFolderSection } from "./ModelsFolderSection";
import { UpdateSection } from "./UpdateSection";

const VRAM_CHOICES = [4, 6, 8, 12, 16, 24];
/** Settings `textEncoderOnCpu` (Rust error messages and notes use the same words). */
const TE_ON_CPU_LABEL = "Read the prompt on the processor";

export function SettingsSheet(props: { open: boolean; onClose: () => void }) {
  const { open, onClose } = props;
  const [saved, setSaved] = useState<"idle" | "saving" | "saved">("idle");
  // The body (and its timer that hides "Saved") unmounts on close: start each visit clean.
  useEffect(() => {
    if (open) setSaved("idle");
  }, [open]);

  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={
        <span className="flex items-center gap-3">
          Settings
          {saved === "saving" && <Spinner className="h-3 w-3 text-neutral-400" />}
          {saved === "saved" && (
            <span className="inline-flex items-center gap-1 text-xs font-normal text-emerald-600 dark:text-emerald-400" aria-live="polite">
              <Check className="h-3.5 w-3.5" /> Saved
            </span>
          )}
        </span>
      }
    >
      {open && <SettingsBody onSaveState={setSaved} />}
    </Sheet>
  );
}

function SettingsBody({ onSaveState }: { onSaveState: (s: "idle" | "saving" | "saved") => void }) {
  const [settings, setLocal] = useState<Settings | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [hw, setHw] = useState<HardwareView | null>(null);
  const [keySet, setKeySet] = useState<boolean | null>(null);
  const [keyDialog, setKeyDialog] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  const engine = useEngine();
  const refreshEngine = engine.refresh;

  const latest = useRef<Settings | null>(null);
  const inflight = useRef(false);
  const dirty = useRef(false);
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadHw = useCallback(() => {
    getHardware()
      .then(setHw)
      .catch((e) => setError(asCoreError(e)));
  }, []);

  useEffect(() => {
    getSettings()
      .then((s) => {
        latest.current = s;
        setLocal(s);
      })
      .catch((e) => setError(asCoreError(e)));
    appInfo()
      .then(setInfo)
      .catch(() => undefined);
    hasCivitaiKey()
      .then(setKeySet)
      .catch(() => setKeySet(null));
    loadHw();
    return () => {
      if (savedTimer.current) clearTimeout(savedTimer.current);
    };
  }, [loadHw]);
  useTauriEvent(onHardwareReady, loadHw);

  const hwDirty = useRef(false);

  // Saves the latest settings; changes made while a save is in flight are coalesced.
  const flush = useCallback(async () => {
    if (inflight.current) {
      dirty.current = true;
      return;
    }
    inflight.current = true;
    onSaveState("saving");
    try {
      do {
        dirty.current = false;
        const result = await saveSettings(latest.current!);
        if (!dirty.current) {
          latest.current = result;
          setLocal(result);
        }
        emitSettingsChanged(result);
      } while (dirty.current);
      setError(null);
      onSaveState("saved");
      if (savedTimer.current) clearTimeout(savedTimer.current);
      savedTimer.current = setTimeout(() => onSaveState("idle"), 1800);
    } catch (e) {
      setError(asCoreError(e));
      onSaveState("idle");
      // Show what is really stored.
      getSettings()
        .then((s) => {
          latest.current = s;
          setLocal(s);
        })
        .catch(() => undefined);
    } finally {
      inflight.current = false;
      if (hwDirty.current) {
        // GPU / VRAM / backend changed: show the new tier and engine state.
        hwDirty.current = false;
        loadHw();
        void refreshEngine();
      }
    }
  }, [refreshEngine, loadHw, onSaveState]);

  const helperModels = useHelperModels();
  const update = (patch: Partial<Settings>) => {
    if (!latest.current) return;
    const next = { ...latest.current, ...patch };
    latest.current = next;
    setLocal(next);
    // (The text-encoder choice changes the engine's note too.)
    if ("gpu" in patch || "vramOverrideGb" in patch || "engineBackend" in patch || "textEncoderOnCpu" in patch) hwDirty.current = true;
    void flush();
  };

  const removeKey = async () => {
    try {
      await clearCivitaiKey();
      setKeySet(false);
    } catch (e) {
      setError(asCoreError(e));
    }
  };

  if (!settings)
    return error ? (
      <ErrorNotice error={error} />
    ) : (
      <div className="space-y-4">
        {[0, 1, 2, 3].map((i) => (
          <Skeleton key={i} className="h-16 w-full" />
        ))}
      </div>
    );

  const gpus = hw?.detected?.gpus ?? [];
  const autoGpu = gpus.slice().sort((a, b) => b.vramGb - a.vramGb)[0];
  const gpuOptions = [
    { value: "auto", label: autoGpu ? `Automatic — ${autoGpu.name}` : "Automatic" },
    ...gpus.map((g) => ({ value: `gpu:${g.index}`, label: `${g.name} (${formatGb(g.vramGb)})` })),
    { value: "cpu", label: "No graphics card — use the processor (slow)" },
  ];
  if (!gpuOptions.some((o) => o.value === settings.gpu)) gpuOptions.splice(1, 0, { value: settings.gpu, label: `Graphics card ${settings.gpu.replace("gpu:", "#")}` });

  const detectedVram = hw?.gpu ? hw.detected?.gpus.find((g) => g.index === hw.gpu?.index)?.vramGb ?? null : null;
  const vramOptions = [
    { value: "auto", label: detectedVram != null ? `Auto (${formatGb(detectedVram)})` : "Auto" },
    ...VRAM_CHOICES.map((n) => ({ value: String(n), label: `${n} GB` })),
  ];
  if (settings.vramOverrideGb != null && !VRAM_CHOICES.includes(settings.vramOverrideGb))
    vramOptions.push({ value: String(settings.vramOverrideGb), label: `${formatGb(settings.vramOverrideGb)}` });

  const st = engine.status;
  const wantBackend = hw?.backend ?? null;
  // engine_status always describes the build for the CURRENT backend (st.backend): after
  // switching backend it reports "not installed" until that build is downloaded, and
  // install_engine is a no-op for a build that is already installed (no "Reinstall").

  return (
    <div className="-mt-1 divide-y divide-neutral-200 dark:divide-neutral-800">
      {error && (
        <div className="pb-4">
          <ErrorNotice error={error} onDismiss={() => setError(null)} />
        </div>
      )}

      <Section title="Privacy">
        <Row
          label="Offline mode"
          hint="Blocks all internet access. Browsing CivitAI and downloads stop until you turn it off."
          control={<Toggle checked={settings.offline} onChange={(v) => update({ offline: v })} label={<span className="sr-only">Offline mode</span>} />}
        />
        <div className="rounded-lg bg-neutral-100 p-3 text-xs text-neutral-600 dark:bg-neutral-800/60 dark:text-neutral-400">
          <p className="flex items-start gap-2">
            <ShieldCheck className="mt-px h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
            <span>
              Pinhole never saves your prompts and sends no usage data. It only goes online when you browse CivitAI, start a download or check for updates. Pictures stay in memory
              until you click Save.
            </span>
          </p>
          <p className="mt-2 pl-6">
            Honest limitation: when memory runs low, your operating system may move parts of it to disk (swap or pagefile). Pinhole can't control that.
          </p>
        </div>
      </Section>

      <Section title="Data folder">
        {info ? (
          <>
            <div className="flex items-center gap-2">
              <Badge tone={info.portable ? "amber" : "neutral"}>{info.portable ? "Portable" : "Installed"}</Badge>
              <span className="text-xs text-neutral-500">{info.portable ? "Stored next to the app — take it anywhere." : "Stored in your user profile."}</span>
            </div>
            <p className="rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 font-mono text-xs break-all text-neutral-700 dark:border-neutral-800 dark:bg-neutral-950 dark:text-neutral-300">
              {info.dataDir}
            </p>
          </>
        ) : (
          <Skeleton className="h-9 w-full" />
        )}
        <Button size="sm" onClick={() => void openDataFolder().catch((e) => setError(asCoreError(e)))}>
          <FolderOpen className="h-4 w-4" /> Open Data folder
        </Button>
      </Section>

      <Section title="Models folder">
        <ModelsFolderSection />
      </Section>

      <Section title="Graphics card">
        <Labeled label="Use this graphics card">
          <Select className="w-full" label="Graphics card" value={settings.gpu} onChange={(gpu) => update({ gpu })} options={gpuOptions} />
        </Labeled>
        <Labeled label="Graphics memory (VRAM)" hint="Only change this if Pinhole reads your graphics memory wrong.">
          <Select
            className="w-full"
            label="Graphics memory"
            value={settings.vramOverrideGb == null ? "auto" : String(settings.vramOverrideGb)}
            onChange={(v) => update({ vramOverrideGb: v === "auto" ? null : Number(v) })}
            options={vramOptions}
          />
        </Labeled>
        {settings.vramOverrideGb != null && hw?.backend === "cpu" && (
          <p className="flex items-start gap-1.5 text-xs text-amber-800 dark:text-amber-300" role="note">
            <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
            <span>
              Pinhole is using the processor, so this graphics memory setting is ignored. It applies once a graphics card is used — pick one above or an
              engine version below.
            </span>
          </p>
        )}
        {hw ? (
          hw.detected ? (
            <p className="text-xs text-neutral-600 dark:text-neutral-400">
              {!isCpuOnly(hw) ? (
                <>
                  Pinhole plans for <b className="text-neutral-900 dark:text-neutral-100">{formatGb(hw.vramGb)}</b> — {tierPlain(hw.tier)}. Model choices, speed and
                  memory-saving options follow this.
                </>
              ) : (
                <>Pinhole will use the processor. Pictures will be slow.</>
              )}
            </p>
          ) : (
            <p className="flex items-center gap-2 text-xs text-neutral-500">
              <Spinner className="h-3 w-3" /> Still checking your hardware…
            </p>
          )
        ) : null}
      </Section>

      <Section title="Engine">
        <Labeled label="Engine version">
          <Select
            className="w-full"
            label="Engine backend"
            value={settings.engineBackend}
            onChange={(engineBackend) => update({ engineBackend })}
            options={[
              { value: "auto", label: `Automatic${wantBackend ? ` (${backendShort(wantBackend)})` : ""} — recommended` },
              { value: "cuda", label: "CUDA — NVIDIA graphics cards" },
              { value: "vulkan", label: "Vulkan — AMD, Intel and NVIDIA" },
              { value: "cpu", label: "CPU — no graphics card (slow)" },
            ]}
          />
        </Labeled>
        {/* Only with a graphics card (known once hardware detection has finished). */}
        {hw && !isCpuOnly(hw) && (
          <Labeled
            label={TE_ON_CPU_LABEL}
            hint="Before the picture is made, your prompt is read on the graphics card. Automatic moves this step to the processor for a model when the card runs out of memory (until Pinhole closes). On always uses the processor: more graphics memory for the picture, a little slower. Off never moves it automatically (a few models always read the prompt on the processor)."
          >
            <Segmented<Settings["textEncoderOnCpu"]>
              size="sm"
              ariaLabel={TE_ON_CPU_LABEL}
              options={[
                { value: "auto", label: "Automatic" },
                { value: "on", label: "On" },
                { value: "off", label: "Off" },
              ]}
              value={settings.textEncoderOnCpu}
              onChange={(textEncoderOnCpu) => update({ textEncoderOnCpu })}
            />
          </Labeled>
        )}
        {st?.note && (
          <p className="flex items-start gap-1.5 text-xs text-neutral-600 dark:text-neutral-400" role="note">
            <Info className="mt-px h-3.5 w-3.5 shrink-0" />
            <span>{st.note}</span>
          </p>
        )}
        <div className="rounded-lg border border-neutral-200 p-3 dark:border-neutral-800">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0 text-sm">
              {!st ? (
                <Skeleton className="h-4 w-40" />
              ) : st.installed ? (
                <span className="inline-flex items-center gap-1.5 font-medium text-emerald-700 dark:text-emerald-400">
                  <CircleCheck className="h-4 w-4" /> Installed · {backendShort(st.backend)}
                </span>
              ) : (
                <span className="inline-flex items-center gap-1.5 font-medium text-amber-700 dark:text-amber-400">
                  <TriangleAlert className="h-4 w-4" /> Not installed{st.backend ? ` · ${backendShort(st.backend)}` : ""}
                </span>
              )}
              {st?.installed && st.version && <div className="text-xs text-neutral-500">Version {st.version}</div>}
              {st && !st.installed && !engine.busy && <div className="text-xs text-neutral-500">Pinhole can't make pictures until the engine is installed.</div>}
            </div>
            {st && !st.installed && !engine.busy && (
              <Button size="sm" variant="primary" onClick={() => void engine.install()}>
                <Download className="h-3.5 w-3.5" />
                Install engine
              </Button>
            )}
          </div>
          {engine.busy && (
            <div className="mt-3">
              {engine.group ? (
                <GroupProgress group={engine.group} onCancel={() => engine.group && void cancelGroup(engine.group.groupId)} />
              ) : (
                <span className="flex items-center gap-2 text-xs text-neutral-500">
                  <Spinner className="h-3 w-3" /> Starting download…
                </span>
              )}
            </div>
          )}
          {engine.error && (
            <div className="mt-3">
              <ErrorNotice error={engine.error} onDismiss={() => engine.setError(null)} />
            </div>
          )}
          {st?.installed && <EngineOutput />}
        </div>
      </Section>

      <Section title="Browsing CivitAI">
        <Labeled label="Safe mode" hint="On hides models made for adults. Turning it off still asks you to confirm once per session.">
          <Segmented<ContentMode>
            size="sm"
            ariaLabel="Safe mode"
            options={[
              { value: "safe", label: "On" },
              { value: "all", label: "Off" },
            ]}
            value={settings.contentMode}
            onChange={(contentMode) => update({ contentMode })}
          />
        </Labeled>
        <Row
          label="Show paid (early access) models"
          hint="Early-access models cost money on CivitAI. They're hidden unless this is on."
          control={<Toggle checked={settings.showPaid} onChange={(v) => update({ showPaid: v })} label={<span className="sr-only">Show paid models</span>} />}
        />
        <Row
          label="Show tips"
          hint="One short tip about a feature you may have missed, under a picture. At most one per session."
          control={<Toggle checked={settings.showTips ?? true} onChange={(v) => update({ showTips: v })} label={<span className="sr-only">Show tips</span>} />}
        />
        <Row
          label="Add trigger words automatically"
          hint="Style add-ons often need a word or two in the prompt to work. Pinhole adds them for you, in memory only. Pick which ones on the add-on’s chip under the prompt."
          control={<Toggle checked={settings.addTriggerWords} onChange={(v) => update({ addTriggerWords: v })} label={<span className="sr-only">Add trigger words automatically</span>} />}
        />
      </Section>

      <Section title="Helper models">
        {(["describe", "improve"] as const).map((purpose) => {
          const key = purpose === "improve" ? "improveModel" : "describeModel";
          const label = purpose === "improve" ? "Improve model" : "Describe model";
          const installed = (helperModels ?? []).filter((m) => m.installed);
          const value = installed.some((m) => m.id === settings[key]) ? settings[key] : "auto";
          return (
            <Labeled
              key={purpose}
              label={label}
              hint={purpose === "improve" ? "The language model that writes the fuller prompt. Automatic uses the larger model when it is installed." : "The language model that describes pictures. Automatic uses the larger model when it is installed."}
            >
              <Select
                label={label}
                value={value}
                onChange={(v) => update({ [key]: v })}
                options={[{ value: "auto", label: "Automatic" }, ...installed.map((m) => ({ value: m.id, label: m.title }))]}
              />
            </Labeled>
          );
        })}
        <p className="text-xs text-neutral-500">Get more helper models on Models → Helpers.</p>
      </Section>

      <Section title="Saved pictures">
        <Labeled
          label="Information inside saved pictures"
          hint={
            settings.savedMetadata === "settings"
              ? "A “made with AI” note, plus model name, seed, steps and dials. Your prompt is never included."
              : "Only a “made with AI” note (always added to pictures Pinhole made). Your prompt is never included."
          }
        >
          <Segmented<"none" | "settings">
            size="sm"
            ariaLabel="Information inside saved pictures"
            options={[
              { value: "none", label: "None" },
              { value: "settings", label: "Settings (no prompt)" },
            ]}
            value={settings.savedMetadata}
            onChange={(savedMetadata) => update({ savedMetadata })}
          />
        </Labeled>
        <Button size="sm" variant="ghost" onClick={() => void openOutputsFolder().catch((e) => setError(asCoreError(e)))}>
          <FolderOpen className="h-4 w-4" /> Open saved pictures folder
        </Button>
      </Section>

      <Section title="CivitAI API key">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0 text-sm">
            {keySet === null ? (
              <Skeleton className="h-4 w-32" />
            ) : keySet ? (
              <span className="inline-flex items-center gap-1.5 font-medium text-emerald-700 dark:text-emerald-400">
                <KeyRound className="h-4 w-4" /> Saved in your system keychain
              </span>
            ) : (
              <span className="text-neutral-700 dark:text-neutral-300">Not set</span>
            )}
            <p className="text-xs text-neutral-500">Optional. Only needed for models that require signing in to CivitAI. Never stored in the Data folder.</p>
          </div>
          <div className="flex shrink-0 gap-1.5">
            {keySet && (
              <Button size="sm" variant="ghost" onClick={() => void removeKey()}>
                Remove
              </Button>
            )}
            <Button size="sm" onClick={() => setKeyDialog(true)}>
              {keySet ? "Replace" : "Add key"}
            </Button>
          </div>
        </div>
      </Section>

      <Section title="Updates">
        <UpdateSection offline={settings.offline} />
      </Section>

      <Section title="Keyboard shortcuts">
        <ShortcutsList />
      </Section>

      <Section title="Appearance">
        <Labeled label="Theme">
          <Segmented<Settings["theme"]>
            size="sm"
            ariaLabel="Theme"
            options={[
              { value: "system", label: "System" },
              { value: "light", label: "Light" },
              { value: "dark", label: "Dark" },
            ]}
            value={settings.theme}
            onChange={(theme) => update({ theme })}
          />
        </Labeled>
      </Section>

      <div className="pt-5 pb-2 text-center text-xs text-neutral-400">Pinhole {info ? `v${info.version}` : ""} · MIT license · No telemetry</div>

      <ApiKeyDialog
        open={keyDialog}
        onClose={() => setKeyDialog(false)}
        onSaved={() => {
          setKeyDialog(false);
          setKeySet(true);
        }}
      />
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="space-y-3 py-5 first:pt-2">
      <h3 className="text-xs font-semibold tracking-wide text-neutral-500 uppercase">{title}</h3>
      {children}
    </section>
  );
}

function Row({ label, hint, control }: { label: string; hint?: string; control: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <div className="text-sm font-medium text-neutral-800 dark:text-neutral-200">{label}</div>
        {hint && <p className="text-xs text-neutral-500">{hint}</p>}
      </div>
      <div className="shrink-0 pt-0.5">{control}</div>
    </div>
  );
}

function Labeled({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="space-y-1.5">
      <div className="text-sm font-medium text-neutral-800 dark:text-neutral-200">{label}</div>
      {children}
      {hint && <p className="text-xs text-neutral-500">{hint}</p>}
    </div>
  );
}
