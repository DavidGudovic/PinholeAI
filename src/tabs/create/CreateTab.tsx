// Create (txt2img), SPEC §5.1.
import { useEffect, useMemo, useRef, useState } from "react";
import { Check, Download, Layers, Sparkles, TriangleAlert, X } from "lucide-react";
import { LiveJobProgress } from "../../components/JobProgress";
import { ErrorWithFix } from "../../components/ErrorWithFix";
import { ModelPicker } from "../../components/ModelPicker";
import { Button, ErrorNotice, IconButton, Kbd, Spinner } from "../../components/ui";
import { RecommendedCards } from "../../firstrun/RecommendedCards";
import { useHardware } from "../models/lib/hooks";
import { isCpuOnly, machinePlain } from "../models/lib/words";
import * as api from "../../lib/api";
import type { CoreError } from "../../lib/types";
import { useActions, usePrimaryAction } from "../../lib/state/AppProvider";
import { useFamilyUi, useModel } from "../../lib/state/hooks";
import { createModels, isActiveDownload } from "../../lib/state/model";
import { modKey } from "../../lib/state/platform";
import { useAppState, useDispatch, useStore } from "../../lib/state/store";
import { Dials } from "./Dials";
import { FineTuneDrawer } from "./FineTune";
import { PasteDialog, PasteSummary } from "./PasteDialog";
import { onGenerationHandoff } from "./handoff";
import { applyPastedText, type PasteOutcome } from "./pasteApply";
import { PresetPicker, type PresetNotice } from "./PresetPicker";
import { PromptBox } from "./PromptBox";
import { Results } from "./Results";

export function CreateTab() {
  const models = useAppState((s) => s.models);
  const usable = useMemo(() => createModels(models), [models]);

  if (models === null) {
    return (
      <div className="flex h-full items-center justify-center text-neutral-500">
        <Spinner />
      </div>
    );
  }
  if (!usable.length) return <NoModels />;
  return <CreateWorkspace />;
}

function NoModels() {
  const actions = useActions();
  const hw = useHardware();
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl px-6 py-10">
        <div className="mb-6 text-center">
          <div className="mx-auto mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300">
            <Sparkles className="h-6 w-6" />
          </div>
          <h1 className="text-xl font-semibold tracking-tight">Get a model to start creating</h1>
          <p className="mt-1 text-sm text-neutral-500">
            Pick one that fits {machinePlain(hw)}. It downloads once and works offline from then on.
            {hw?.detected && isCpuOnly(hw) ? " Without a graphics card, each picture takes a few minutes." : ""}
          </p>
        </div>
        <RecommendedCards roles={["realistic", "anime"]} compact />
        <div className="mt-6 text-center">
          <Button variant="ghost" onClick={() => actions.setTab("models")}>
            <Layers className="h-4 w-4" /> Browse all models
          </Button>
        </div>
      </div>
    </div>
  );
}

const CREATE_JOBS = ["create", "upscale"] as const;

function CreateWorkspace() {
  const models = useAppState((s) => s.models);
  const modelId = useAppState((s) => s.create.modelId);
  // Only the kind: the progress card subscribes to the job itself (LiveJobProgress).
  const jobKind = useAppState((s) => s.job?.kind ?? null);
  const dispatch = useDispatch();
  const store = useStore();
  const actions = useActions();
  const usable = useMemo(() => createModels(models), [models]);
  const model = useModel(modelId);
  const ui = useFamilyUi(model?.familyId);

  const [error, setError] = useState<CoreError | null>(null);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [outcome, setOutcome] = useState<PasteOutcome | null>(null);
  const [presetNotice, setPresetNotice] = useState<PresetNotice | null>(null);
  const [cancelling, setCancelling] = useState(false);

  // Set before the first await, so a second press while the job is starting is ignored.
  const generating = useRef(false);
  const generate = async () => {
    if (generating.current || store.getState().job) return;
    generating.current = true;
    setError(null);
    try {
      await actions.generateCreate();
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      generating.current = false;
    }
  };
  usePrimaryAction("create", () => void generate());

  const cancel = async () => {
    setCancelling(true);
    await actions.cancel();
    setCancelling(false);
  };

  const applyPaste = async (text: string) => {
    const o = await applyPastedText(text, store, actions);
    setOutcome(o);
    setPresetNotice(null);
    setError(null);
  };

  // "Use these settings" from a model's details page.
  const applyPasteRef = useRef(applyPaste);
  applyPasteRef.current = applyPaste;
  useEffect(() => onGenerationHandoff((t) => void applyPasteRef.current(t).catch((e) => setError(api.asCoreError(e)))), []);

  const myJob = jobKind === "create" || jobKind === "upscale";

  return (
    <div className="grid h-full grid-cols-[minmax(360px,420px)_minmax(0,1fr)]">
      <aside aria-label="Create settings" className="flex min-h-0 flex-col border-r border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pt-4 pb-5">
          <div className="flex items-stretch gap-2">
            <div className="min-w-0 flex-1">
              <ModelPicker models={usable} value={modelId} onChange={(id) => dispatch({ type: "selectModel", modelId: id })} />
            </div>
            <PresetPicker onApplied={setPresetNotice} />
          </div>
          {model && model.missingComponents.length > 0 && (
            <p className="flex items-start gap-1.5 text-xs text-amber-800 dark:text-amber-300">
              <TriangleAlert className="mt-px h-3.5 w-3.5 shrink-0" />
              This model still needs {model.missingComponents.join(", ")}. Open Models → Installed to finish setting it up.
            </p>
          )}
          {ui?.licenseNote && model && /non-commercial/i.test(ui.licenseNote) && (
            <p className="text-xs text-neutral-500">{model.friendlyName}: {ui.licenseNote}</p>
          )}
          {presetNotice && <PresetNoticeCard notice={presetNotice} onDismiss={() => setPresetNotice(null)} />}

          <PromptBox ui={ui} onOpenPaste={() => setPasteOpen(true)} onApplyPasted={(t) => void applyPaste(t).catch((e) => setError(api.asCoreError(e)))} />
          {outcome && <PasteSummary outcome={outcome} onDismiss={() => setOutcome(null)} onOutcome={setOutcome} />}

          <Dials ui={ui} />
          <FineTuneDrawer ui={ui} model={model} />
        </div>

        <div className="shrink-0 space-y-2 border-t border-neutral-200 bg-white px-5 py-4 dark:border-neutral-800 dark:bg-neutral-900">
          {myJob ? (
            <LiveJobProgress kinds={CREATE_JOBS} onCancel={() => void cancel()} cancelling={cancelling} />
          ) : (
            <Button variant="primary" size="lg" className="w-full" disabled={!!jobKind || !model} onClick={() => void generate()}>
              <Sparkles className="h-4 w-4" />
              Generate
              <span className="ml-1 inline-flex gap-0.5 opacity-70">
                <Kbd>{modKey}</Kbd>
                <Kbd>Enter</Kbd>
              </span>
            </Button>
          )}
          {jobKind && !myJob && <p className="text-center text-xs text-neutral-500">Busy with an edit — Generate is available when it finishes.</p>}
          {error && <ErrorWithFix error={error} onDismiss={() => setError(null)} onRetry={() => void generate()} />}
        </div>
      </aside>

      <div className="min-h-0 min-w-0 bg-neutral-100 dark:bg-neutral-950">
        <div className="flex h-full min-h-0 flex-col">
          <Results />
        </div>
      </div>

      <PasteDialog open={pasteOpen} onClose={() => setPasteOpen(false)} onApply={applyPaste} />
    </div>
  );
}

export function PresetNoticeCard({ notice, onDismiss }: { notice: PresetNotice; onDismiss: () => void }) {
  const actions = useActions();
  const [starting, setStarting] = useState(false);
  const [groupId, setGroupId] = useState<string | null>(null);
  // Follow the download it started, so the button can say what happened (like PasteSummary).
  const dl = useAppState((s) => (groupId ? s.downloads.find((d) => d.groupId === groupId) : undefined));
  const [error, setError] = useState<CoreError | null>(null);
  const { preset, app } = notice;
  const vid = app.missingModel?.civitaiVersionId ?? null;
  const downloading = !!dl && isActiveDownload(dl);
  const failed = !!dl && !downloading && dl.state !== "done";
  const getModel = async () => {
    if (vid == null || starting) return;
    setError(null);
    setStarting(true);
    try {
      const started = await api.installCivitai(vid, app.missingModel?.family ?? null);
      setGroupId(started.groupId);
      await actions.refreshDownloads().catch(() => undefined);
    } catch (e) {
      setError(api.asCoreError(e));
    } finally {
      setStarting(false);
    }
  };
  return (
    <div className="rounded-xl border border-amber-200 bg-amber-50/70 p-3 text-sm dark:border-amber-500/20 dark:bg-amber-500/5">
      <div className="flex items-start gap-2">
        <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        <div className="min-w-0 flex-1 space-y-1">
          <div className="font-medium">“{preset.name}” is partly applied</div>
          {app.missingModel && (
            <p className="text-xs text-neutral-600 dark:text-neutral-400">
              It was made for a model you don’t have{app.missingModel.family ? ` (${app.missingModel.family.replace(/_/g, " ")})` : ""}. Your current model is kept.
            </p>
          )}
          {app.missingLoras.length > 0 && (
            <p className="text-xs text-neutral-600 dark:text-neutral-400">Missing add-ons: {app.missingLoras.map((l) => l.name).join(", ")}</p>
          )}
          {app.missingStyle && <p className="text-xs text-neutral-600 dark:text-neutral-400">Its style was deleted; your style is kept.</p>}
          <div className="flex flex-wrap gap-2 pt-1">
            {app.missingModel && vid != null && dl?.state === "done" && (
              <span className="inline-flex items-center gap-1 text-xs text-emerald-700 dark:text-emerald-400">
                <Check className="h-3.5 w-3.5" /> Downloaded. Pick it in the model list.
              </span>
            )}
            {app.missingModel && vid != null && dl?.state !== "done" && (
              <Button size="sm" variant="primary" disabled={starting || downloading} onClick={() => void getModel()}>
                {starting ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-3.5 w-3.5" />}{" "}
                {downloading ? "Downloading…" : failed ? "Try again" : "Get the model"}
              </Button>
            )}
            {app.missingModel && vid == null && (
              <Button size="sm" onClick={() => actions.setTab("models")}>
                <Layers className="h-3.5 w-3.5" /> Find a model
              </Button>
            )}
          </div>
          {error && <ErrorNotice error={error} />}
        </div>
        <IconButton label="Dismiss" size="sm" onClick={onDismiss} className="-mt-1 -mr-1">
          <X className="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  );
}
