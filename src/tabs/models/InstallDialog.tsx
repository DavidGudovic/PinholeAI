// Install flow for a CivitAI version (SPEC §5.4 "Install"): plan → review → download.
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { CircleCheck, Download, FileBox, HardDrive, KeyRound, Puzzle, ShieldAlert, TriangleAlert } from "lucide-react";
import { asCoreError, getSettings, installCivitai, planCivitaiInstall, setSettings } from "../../lib/api";
import type { CoreError, InstallPlan, PlanFileOption, Settings } from "../../lib/types";
import { formatBytes } from "../../lib/format";
import { Badge, Button, Dialog, ErrorNotice, Spinner, Toggle, VramBadge } from "../../components/ui";
import { emitSettingsChanged } from "../../settings/events";
import { ApiKeyDialog, FamilyPicker, Skeleton } from "./controls";
import { tagGroup } from "./lib/downloads";
import { formatLabel } from "./lib/words";

export function InstallDialog({ versionId, title, onClose }: { versionId: number | null; title?: string; onClose: () => void }) {
  const [plan, setPlan] = useState<InstallPlan | null>(null);
  const [error, setError] = useState<CoreError | null>(null);
  const [family, setFamily] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [keyOpen, setKeyOpen] = useState(false);
  const [keyReason, setKeyReason] = useState<string | null>(null);
  const [keyThenInstall, setKeyThenInstall] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const [settings, setLocalSettings] = useState<Settings | null>(null);
  // The size the user picked (a CivitAI file id), for this version only. Null = Pinhole's pick.
  const [choice, setChoice] = useState<{ versionId: number; fileId: number } | null>(null);
  const chosenFile = choice && choice.versionId === versionId ? choice.fileId : null;

  useEffect(() => {
    if (versionId == null) return;
    let alive = true;
    setPlan(null);
    setError(null);
    if (attempt === 0) setKeyOpen(false);
    planCivitaiInstall(versionId, chosenFile)
      .then((p) => {
        if (!alive) return;
        setPlan(p);
        setFamily(p.familyCandidates.length ? null : (p.family?.familyId ?? null));
      })
      .catch((e) => {
        if (!alive) return;
        const ce = asCoreError(e);
        setError(ce);
        if (ce.code === "unauthorized") {
          setKeyReason(ce.message);
          setKeyThenInstall(false);
          setKeyOpen(true);
        }
      });
    return () => {
      alive = false;
    };
  }, [versionId, attempt, chosenFile]);

  useEffect(() => {
    if (versionId == null) return;
    getSettings()
      .then(setLocalSettings)
      .catch(() => undefined);
  }, [versionId]);

  const install = useCallback(
    async (skipKeyCheck = false) => {
      if (!plan) return;
      if (plan.needsApiKey && !skipKeyCheck) {
        setKeyReason("CivitAI only lets signed-in users download this model.");
        setKeyThenInstall(true);
        setKeyOpen(true);
        return;
      }
      setInstalling(true);
      setError(null);
      try {
        const familyId = plan.familyCandidates.length ? family : (plan.family?.familyId ?? null);
        const fileId = plan.fileOptions?.find((o) => o.selected)?.fileId ?? null;
        const { groupId } = await installCivitai(plan.versionId, familyId, fileId);
        tagGroup(`civitai:${plan.versionId}`, groupId);
        onClose();
      } catch (e) {
        const ce = asCoreError(e);
        if (ce.code === "unauthorized") {
          setKeyReason(ce.message);
          setKeyThenInstall(true);
          setKeyOpen(true);
        } else setError(ce);
      } finally {
        setInstalling(false);
      }
    },
    [plan, family, onClose],
  );

  const setTriggerWords = async (v: boolean) => {
    if (!settings) return;
    const prev = settings;
    setLocalSettings({ ...settings, addTriggerWords: v });
    try {
      const saved = await setSettings({ ...prev, addTriggerWords: v });
      setLocalSettings(saved);
      emitSettingsChanged(saved);
    } catch (e) {
      setLocalSettings(prev);
      setError(asCoreError(e));
    }
  };

  const open = versionId != null;
  const blocked = !!plan?.blockedReason;
  const needsFamily = !!plan && plan.familyCandidates.length > 0 && !family;
  const tooBig = plan?.fit === "tooBig";
  const canInstall = !!plan && !blocked && plan.enoughDisk && !needsFamily && !installing;

  const footer = (
    <>
      <Button variant="ghost" onClick={onClose}>
        {blocked ? "Close" : "Cancel"}
      </Button>
      {!blocked && (
        <Button variant={tooBig ? "danger" : "primary"} onClick={() => void install()} disabled={!canInstall}>
          {installing ? <Spinner className="h-3.5 w-3.5" /> : <Download className="h-4 w-4" />}
          {tooBig ? "Download anyway" : plan ? `Install · ${formatBytes(plan.totalDownloadBytes)}` : "Install"}
        </Button>
      )}
    </>
  );

  return (
    <>
      <Dialog
        open={open && !keyOpen}
        onClose={onClose}
        title={`Install ${plan?.modelName ?? title ?? "model"}`}
        description={plan ? `${plan.versionName}${plan.family ? ` · ${plan.family.label}` : ""}${plan.isLora ? " · Style add-on" : ""}` : undefined}
        footer={footer}
      >
        {!plan && !error && <PlanSkeleton />}
        {!plan && error && <ErrorNotice error={error} />}
        {plan && (
          <div className="space-y-5 text-sm">
            {plan.blockedReason && (
              <div className="flex items-start gap-2 rounded-lg border border-red-200 bg-red-50 p-3 text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200">
                <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0" />
                <div>
                  <div className="font-medium">Pinhole won't install this</div>
                  <p className="mt-0.5">{plan.blockedReason}</p>
                </div>
              </div>
            )}

            <Section title="What will be downloaded">
              <ul className="divide-y divide-neutral-100 rounded-lg border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
                <FileRow icon={<FileBox className="h-4 w-4" />} name={plan.mainFile.name} note={formatLabel(plan.mainFile.format)} size={plan.mainFile.sizeBytes} />
                {plan.components.map((c) => (
                  <FileRow key={c.componentId} icon={<Puzzle className="h-4 w-4" />} name={c.label} note={c.installed ? null : "Needed to run it"} size={c.installed ? null : c.sizeBytes} installed={c.installed} />
                ))}
              </ul>
              <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-neutral-600 dark:text-neutral-400">
                <span>
                  Total download: <b className="text-neutral-900 dark:text-neutral-100">{formatBytes(plan.totalDownloadBytes)}</b>
                </span>
                <span className="inline-flex items-center gap-1">
                  <HardDrive className="h-3.5 w-3.5" /> {formatBytes(plan.freeDiskBytes)} free on this drive
                </span>
              </div>
              {!plan.enoughDisk && (
                <Callout tone="red" icon={<HardDrive className="h-4 w-4" />}>
                  Not enough disk space. This needs {formatBytes(plan.totalDownloadBytes)} but only {formatBytes(plan.freeDiskBytes)} is free. Delete models you don't
                  use (Models → Installed) or free up space, then try again.
                </Callout>
              )}
            </Section>

            {!plan.isLora && (plan.fileOptions?.length ?? 0) > 1 && (
              <Section title="Size">
                <SizeChoice
                  options={plan.fileOptions ?? []}
                  smaller={plan.smallerFile ?? null}
                  onPick={(fileId) => versionId != null && setChoice({ versionId, fileId })}
                  disabled={installing}
                />
              </Section>
            )}

            {(plan.vram || !plan.isLora) && (
              <Section title={plan.vram?.onCpu ? "Your computer" : "Your graphics card"}>
                {plan.vram ? <VramBadge vram={plan.vram} fit={plan.fit} /> : <span className="text-xs text-neutral-500">Pinhole will estimate this after the download.</span>}
                {plan.fit === "tight" && (
                  <p className="mt-1.5 text-xs text-neutral-500">
                    {plan.vram?.onCpu
                      ? "Pinhole didn't find a graphics card, so this runs on the processor. Expect a few minutes per picture."
                      : "It fits with memory-saving options, which Pinhole turns on automatically. Pictures take a bit longer."}
                  </p>
                )}
                {tooBig && (
                  <Callout tone="amber" icon={<TriangleAlert className="h-4 w-4" />}>
                    {plan.vram?.onCpu
                      ? "Pinhole didn't find a graphics card, and this model is too big to run on the processor. You can still download it, but it won't run well here."
                      : "This model is probably too big for your graphics card. It may be very slow or fail to load. You can still download it."}
                  </Callout>
                )}
              </Section>
            )}

            {plan.familyCandidates.length > 0 && (
              <Section title="Which kind of model is this?">
                <p className="mb-2 text-xs text-neutral-500">Pinhole couldn't tell for sure. Pick the one the model's CivitAI page mentions.</p>
                <FamilyPicker candidates={plan.familyCandidates} value={family} onChange={setFamily} />
              </Section>
            )}

            {plan.isLora && (
              <Section title="Trigger words">
                {plan.trainedWords.length ? (
                  <>
                    <div className="flex flex-wrap gap-1.5">
                      {plan.trainedWords.map((w) => (
                        <Badge key={w} tone="amber">
                          {w}
                        </Badge>
                      ))}
                    </div>
                    <div className="mt-2.5">
                      <Toggle
                        checked={settings?.addTriggerWords ?? true}
                        onChange={(v) => void setTriggerWords(v)}
                        disabled={!settings}
                        label={<span className="text-sm">Add trigger words automatically</span>}
                      />
                    </div>
                  </>
                ) : (
                  <p className="text-xs text-neutral-500">This style add-on doesn't need special words. It works on its own.</p>
                )}
              </Section>
            )}

            {(plan.licenseNote || plan.needsApiKey) && (
              <div className="space-y-2">
                {plan.licenseNote && (
                  <p className="text-xs text-neutral-500">
                    License: <span className="text-neutral-700 dark:text-neutral-300">{plan.licenseNote}</span>. Check it before using pictures for clients.
                  </p>
                )}
                {plan.needsApiKey && !blocked && (
                  <p className="flex items-start gap-1.5 text-xs text-neutral-600 dark:text-neutral-400">
                    <KeyRound className="mt-px h-3.5 w-3.5 shrink-0 text-amber-500" />
                    CivitAI needs your API key for this download. Pinhole will ask for it when you install.
                  </p>
                )}
              </div>
            )}

            {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
          </div>
        )}
      </Dialog>
      <ApiKeyDialog
        open={open && keyOpen}
        reason={keyReason}
        onClose={() => setKeyOpen(false)}
        onSaved={() => {
          setKeyOpen(false);
          if (keyThenInstall && plan) void install(true);
          else setAttempt((a) => a + 1);
        }}
      />
    </>
  );
}

/**
 * The files of one version at different sizes (full quality, compact FP8, Q4…). Pinhole picks the
 * best one that fits the card; the user can pick another. Plain words, SPEC §5.4 "Install".
 */
function SizeChoice({ options, smaller, onPick, disabled }: { options: PlanFileOption[]; smaller: string | null; onPick: (fileId: number) => void; disabled: boolean }) {
  return (
    <div>
      <p className="mb-2 text-xs text-neutral-500">
        {smaller
          ? `The usual file doesn't fit your graphics card well, so Pinhole picked the “${smaller}” version. `
          : "This model comes in more than one size. "}
        Compact versions are the same model stored with fewer digits per number: they need less graphics memory, pictures keep their size, and fine detail is a
        little softer.
      </p>
      <div role="radiogroup" aria-label="Size" className="divide-y divide-neutral-100 rounded-lg border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
        {options.map((o) => (
          <label key={o.fileId} className="flex cursor-pointer items-center gap-3 px-3 py-2 has-[:disabled]:cursor-default">
            <input type="radio" name="install-size" checked={o.selected} disabled={disabled} onChange={() => onPick(o.fileId)} className="accent-amber-500" />
            <span className="min-w-0 flex-1">
              <span className="block text-neutral-800 dark:text-neutral-200">{o.label}</span>
              <span className="block truncate text-[11px] text-neutral-500" title={o.name}>
                {formatBytes(o.sizeBytes)}
              </span>
            </span>
            <VramBadge vram={o.vram} fit={o.fit} compact />
          </label>
        ))}
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <h3 className="mb-2 text-xs font-semibold tracking-wide text-neutral-500 uppercase">{title}</h3>
      {children}
    </section>
  );
}

function FileRow({ icon, name, note, size, installed }: { icon: ReactNode; name: string; note: string | null; size: number | null; installed?: boolean }) {
  return (
    <li className="flex items-center gap-3 px-3 py-2">
      <span className="text-neutral-400">{icon}</span>
      <span className="min-w-0 flex-1">
        <span className={`block truncate ${installed ? "text-neutral-500" : "text-neutral-800 dark:text-neutral-200"}`} title={name}>
          {name}
        </span>
        {note && <span className="block text-[11px] text-neutral-500">{note}</span>}
      </span>
      {installed ? (
        <span className="inline-flex shrink-0 items-center gap-1 text-xs text-emerald-700 dark:text-emerald-400">
          <CircleCheck className="h-3.5 w-3.5" /> Already installed
        </span>
      ) : (
        <span className="shrink-0 text-xs font-medium text-neutral-700 tabular-nums dark:text-neutral-300">{formatBytes(size)}</span>
      )}
    </li>
  );
}

function Callout({ tone, icon, children }: { tone: "red" | "amber"; icon: ReactNode; children: ReactNode }) {
  const tones = {
    red: "border-red-200 bg-red-50 text-red-900 dark:border-red-900 dark:bg-red-950/40 dark:text-red-200",
    amber: "border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-200",
  };
  return (
    <div className={`mt-2 flex items-start gap-2 rounded-lg border p-2.5 text-xs ${tones[tone]}`}>
      <span className="mt-px shrink-0">{icon}</span>
      <span>{children}</span>
    </div>
  );
}

function PlanSkeleton() {
  return (
    <div className="space-y-3" aria-busy="true">
      <div className="flex items-center gap-2 text-sm text-neutral-500">
        <Spinner className="h-4 w-4 text-amber-500" /> Checking the files on CivitAI…
      </div>
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-10 w-full" />
      <Skeleton className="h-4 w-2/3" />
    </div>
  );
}
