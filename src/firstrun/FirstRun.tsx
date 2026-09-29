// OWNER: frontend B. Full-screen first-run flow (engine download → recommended models).
// Keep this export signature.
import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { ArrowLeft, ArrowRight, Check, CircleCheck, Cpu, Download, Gpu, Lock, MemoryStick, RotateCw, ShieldCheck, Sparkles, TriangleAlert, WifiOff } from "lucide-react";
import { asCoreError, getHardware, getSettings, onHardwareReady, setSettings } from "../lib/api";
import type { CoreError, HardwareView, Settings } from "../lib/types";
import { formatGb } from "../lib/format";
import { Button, ErrorNotice, Spinner } from "../components/ui";
import { Logo } from "../components/Logo";
import { GroupProgress } from "../tabs/models/controls";
import { cancelGroup } from "../tabs/models/lib/downloads";
import { useEngine, useTauriEvent } from "../tabs/models/lib/hooks";
import { backendPlain, isCpuOnly, machinePlain, tierPlain } from "../tabs/models/lib/words";
import { emitSettingsChanged } from "../settings/events";
import { RecommendedCards } from "./RecommendedCards";

type Step = "welcome" | "hardware" | "engine" | "models";
const STEPS: { key: Step; label: string }[] = [
  { key: "welcome", label: "Welcome" },
  { key: "hardware", label: "Your computer" },
  { key: "engine", label: "Engine" },
  { key: "models", label: "Models" },
];

export function FirstRun(props: { onDone: () => void }) {
  const { onDone } = props;
  const [step, setStep] = useState<Step>("welcome");
  const [hw, setHw] = useState<HardwareView | null>(null);
  const [hwError, setHwError] = useState<CoreError | null>(null);
  const [settings, setLocalSettings] = useState<Settings | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [finishError, setFinishError] = useState<CoreError | null>(null);
  const engine = useEngine();
  const headingRef = useRef<HTMLHeadingElement>(null);

  const loadHw = useCallback(async () => {
    try {
      setHw(await getHardware());
      setHwError(null);
    } catch (e) {
      setHwError(asCoreError(e));
    }
  }, []);

  useEffect(() => {
    void loadHw();
    getSettings()
      .then(setLocalSettings)
      .catch(() => undefined);
  }, [loadHw]);
  useTauriEvent(onHardwareReady, () => void loadHw());

  // Fallback in case the hardware-ready event fired before we subscribed.
  const detecting = !hw?.detected && !hwError;
  useEffect(() => {
    if (!detecting) return;
    const t = setTimeout(() => void loadHw(), 2500);
    return () => clearTimeout(t);
  }, [detecting, hw, loadHw]);

  useEffect(() => {
    headingRef.current?.focus();
  }, [step]);

  const idx = STEPS.findIndex((s) => s.key === step);
  const go = (d: number) => setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, idx + d))].key);

  const finish = async () => {
    setFinishing(true);
    setFinishError(null);
    try {
      const current = settings ?? (await getSettings());
      const saved = await setSettings({ ...current, firstRunDone: true });
      emitSettingsChanged(saved);
      onDone();
    } catch (e) {
      setFinishError(asCoreError(e));
    } finally {
      setFinishing(false);
    }
  };

  // No usable GPU (or not detected yet): "your computer", never "your GPU (0 GB)".
  const cpuOnly = isCpuOnly(hw);
  const vramLabel = hw?.detected && !cpuOnly ? formatGb(hw.vramGb) : null;
  const engineReady = !!engine.status?.installed && !engine.busy;

  let footer: ReactNode;
  if (step === "welcome")
    footer = (
      <>
        <Button variant="ghost" onClick={finish} disabled={finishing}>
          Skip setup
        </Button>
        <Button variant="primary" size="lg" onClick={() => go(1)}>
          Get started <ArrowRight className="h-4 w-4" />
        </Button>
      </>
    );
  else if (step === "hardware")
    footer = (
      <>
        <BackButton onClick={() => go(-1)} />
        <Button variant="primary" size="lg" onClick={() => go(1)}>
          Continue <ArrowRight className="h-4 w-4" />
        </Button>
      </>
    );
  else if (step === "engine")
    footer = (
      <>
        <BackButton onClick={() => go(-1)} />
        <div className="flex items-center gap-2">
          {!engineReady && !engine.busy && (
            <Button variant="ghost" onClick={() => go(1)}>
              Skip for now
            </Button>
          )}
          <Button variant="primary" size="lg" onClick={() => go(1)} disabled={!engineReady && !engine.busy}>
            {engine.busy ? "Continue while it downloads" : "Continue"} <ArrowRight className="h-4 w-4" />
          </Button>
        </div>
      </>
    );
  else
    footer = (
      <>
        <BackButton onClick={() => go(-1)} />
        <div className="flex items-center gap-2">
          <Button variant="ghost" onClick={finish} disabled={finishing}>
            Skip
          </Button>
          <Button variant="primary" size="lg" onClick={finish} disabled={finishing}>
            {finishing ? <Spinner className="h-4 w-4" /> : <Check className="h-4 w-4" />} Done
          </Button>
        </div>
      </>
    );

  return (
    <div className="fixed inset-0 z-30 overflow-y-auto bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <div className="mx-auto flex min-h-full max-w-4xl flex-col px-6 py-6 sm:px-10">
        <header className="flex items-center justify-between gap-4">
          <div className="flex items-center gap-2 font-semibold">
            <Logo className="h-7 w-7" /> Pinhole
          </div>
          <Stepper current={idx} />
        </header>

        <main className="flex flex-1 flex-col justify-center py-10">
          {step === "welcome" && (
            <section className="mx-auto max-w-xl text-center">
              <Logo className="mx-auto h-20 w-20" />
              <h1 ref={headingRef} tabIndex={-1} className="mt-6 text-3xl font-semibold tracking-tight outline-none">
                Welcome to Pinhole
              </h1>
              <p className="mt-3 text-lg text-neutral-600 dark:text-neutral-400">Make images with AI on your own computer. No account, no cloud.</p>
              <p className="mx-auto mt-6 flex max-w-lg items-start gap-3 rounded-xl border border-emerald-200 bg-emerald-50 px-4 py-3 text-left text-sm text-emerald-900 dark:border-emerald-900/60 dark:bg-emerald-500/10 dark:text-emerald-200">
                <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0" />
                Your prompts and images stay on your computer. Pinhole only goes online when you browse CivitAI, download something or check for updates.
              </p>
              <ul className="mx-auto mt-8 grid max-w-lg gap-4 text-left text-sm text-neutral-600 dark:text-neutral-400">
                <Feature icon={<Lock className="h-4 w-4" />} title="You choose what gets saved">
                  New pictures are kept in memory until you click Save.
                </Feature>
                <Feature icon={<Sparkles className="h-4 w-4" />} title="No expert knowledge needed">
                  Pinhole picks the right models and settings for your computer.
                </Feature>
                <Feature icon={<WifiOff className="h-4 w-4" />} title="Works offline">
                  Once your models are downloaded, you don't need the internet.
                </Feature>
              </ul>
            </section>
          )}

          {step === "hardware" && (
            <section className="mx-auto w-full max-w-xl">
              <StepHeading refObj={headingRef} title="Your computer">
                Pinhole checks your graphics card and memory to pick models that run well on your computer.
              </StepHeading>
              {hwError ? (
                <div className="space-y-3">
                  <ErrorNotice error={hwError} />
                  <Button onClick={() => void loadHw()}>
                    <RotateCw className="h-4 w-4" /> Check again
                  </Button>
                </div>
              ) : !hw?.detected ? (
                <div className="flex items-center gap-3 rounded-xl border border-neutral-200 bg-white p-5 text-sm text-neutral-600 dark:border-neutral-800 dark:bg-neutral-900 dark:text-neutral-400">
                  <Spinner className="h-5 w-5 text-amber-500" /> Checking your computer…
                </div>
              ) : (
                <HardwareSummary hw={hw} />
              )}
            </section>
          )}

          {step === "engine" && (
            <section className="mx-auto w-full max-w-xl">
              <StepHeading refObj={headingRef} title="Download the image engine">
                Pinhole makes pictures with a small open-source engine. It's downloaded once from GitHub and checked before it runs.
              </StepHeading>
              <EngineStep engine={engine} hw={hw} />
            </section>
          )}

          {step === "models" && (
            <section className="w-full">
              <StepHeading refObj={headingRef} title={vramLabel ? `Recommended for your GPU (${vramLabel})` : "Recommended for your computer"}>
                Each pick is the best model that fits {machinePlain(hw)}.{" "}
                {hw?.detected && cpuOnly ? "Without a graphics card, pictures are made by the processor and take a few minutes each. " : ""}Downloads keep going in the
                background, so you can start right away. You can always find more in the Models tab.
              </StepHeading>
              {!engineReady && (
                <p className="mb-4 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-900/60 dark:bg-amber-500/10 dark:text-amber-200">
                  <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
                  {engine.busy
                    ? "The engine is still downloading. Models can download at the same time."
                    : "The image engine isn't installed yet, so Pinhole can't make pictures. You can get it later in Settings."}
                </p>
              )}
              <RecommendedCards showGetAll />
            </section>
          )}
          {finishError && (
            <div className="mx-auto mt-6 w-full max-w-xl space-y-2">
              <ErrorNotice error={finishError} />
              <Button size="sm" onClick={onDone}>
                Continue anyway
              </Button>
            </div>
          )}
        </main>

        <footer className="flex items-center justify-between gap-3 border-t border-neutral-200 pt-4 dark:border-neutral-800">{footer}</footer>
      </div>
    </div>
  );
}

function BackButton({ onClick }: { onClick: () => void }) {
  return (
    <Button variant="ghost" onClick={onClick}>
      <ArrowLeft className="h-4 w-4" /> Back
    </Button>
  );
}

function Feature({ icon, title, children }: { icon: ReactNode; title: string; children: ReactNode }) {
  return (
    <li className="flex gap-3">
      <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-400">{icon}</span>
      <span>
        <span className="block font-medium text-neutral-900 dark:text-neutral-100">{title}</span>
        {children}
      </span>
    </li>
  );
}

function StepHeading({ refObj, title, children }: { refObj: RefObject<HTMLHeadingElement | null>; title: string; children: ReactNode }) {
  return (
    <div className="mb-6">
      <h1 ref={refObj} tabIndex={-1} className="text-2xl font-semibold tracking-tight outline-none">
        {title}
      </h1>
      <p className="mt-2 text-neutral-600 dark:text-neutral-400">{children}</p>
    </div>
  );
}

function Stepper({ current }: { current: number }) {
  return (
    <ol className="flex items-center gap-1.5 text-xs" aria-label="Setup progress">
      {STEPS.map((s, i) => (
        <li key={s.key} className="flex items-center gap-1.5" aria-current={i === current ? "step" : undefined}>
          <span
            className={`flex h-5 w-5 items-center justify-center rounded-full text-[10px] font-semibold ${
              i < current
                ? "bg-amber-500 text-neutral-950"
                : i === current
                  ? "border-2 border-amber-500 text-amber-600 dark:text-amber-400"
                  : "border border-neutral-300 text-neutral-400 dark:border-neutral-700"
            }`}
          >
            {i < current ? <Check className="h-3 w-3" /> : i + 1}
          </span>
          <span className={`hidden sm:inline ${i === current ? "font-medium text-neutral-900 dark:text-neutral-100" : "text-neutral-500"}`}>{s.label}</span>
          {i < STEPS.length - 1 && <span className="mx-1 h-px w-5 bg-neutral-300 dark:bg-neutral-700" />}
        </li>
      ))}
    </ol>
  );
}

function HardwareSummary({ hw }: { hw: HardwareView }) {
  const d = hw.detected!;
  const others = d.gpus.filter((g) => g.index !== hw.gpu?.index);
  return (
    <div className="space-y-3">
      {hw.gpu ? (
        <div className="rounded-xl border border-neutral-200 bg-white p-5 shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
          <div className="flex items-start gap-4">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-emerald-100 text-emerald-700 dark:bg-emerald-500/15 dark:text-emerald-400">
              <Gpu className="h-6 w-6" />
            </span>
            <div className="min-w-0">
              <div className="text-lg font-semibold">{hw.gpu.name}</div>
              <div className="text-sm text-neutral-600 dark:text-neutral-400">
                {formatGb(hw.vramGb)} graphics memory (VRAM) · {tierPlain(hw.tier)}
              </div>
              <div className="mt-1 inline-flex items-center gap-1.5 text-sm font-medium text-emerald-700 dark:text-emerald-400">
                <CircleCheck className="h-4 w-4" /> Good to go
              </div>
            </div>
          </div>
        </div>
      ) : (
        <div className="rounded-xl border border-amber-300 bg-amber-50 p-5 dark:border-amber-900/60 dark:bg-amber-500/10">
          <div className="flex items-start gap-4">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-amber-100 text-amber-700 dark:bg-amber-500/20 dark:text-amber-300">
              <TriangleAlert className="h-6 w-6" />
            </span>
            <div className="text-sm text-amber-900 dark:text-amber-200">
              <div className="text-lg font-semibold">No GPU found — images will be slow</div>
              <p className="mt-1">
                Pinhole will use your processor, which can take a few minutes per picture. If you have a graphics card, update its driver and restart Pinhole.
              </p>
            </div>
          </div>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3 text-sm">
        <Stat icon={<MemoryStick className="h-4 w-4" />} label="Memory (RAM)" value={formatGb(d.ramGb)} />
        <Stat icon={<Cpu className="h-4 w-4" />} label="Processor" value={`${d.cpuThreads} threads`} />
      </div>
      {others.length > 0 && hw.gpu && (
        <p className="text-xs text-neutral-500">
          Also found: {others.map((g) => g.name).join(", ")}. Pinhole uses the {hw.gpu.name}; you can change this in Settings.
        </p>
      )}
    </div>
  );
}

function Stat({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-neutral-200 bg-white px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900">
      <span className="text-neutral-400">{icon}</span>
      <span>
        <span className="block text-xs text-neutral-500">{label}</span>
        <span className="font-medium">{value}</span>
      </span>
    </div>
  );
}

function EngineStep({ engine, hw }: { engine: ReturnType<typeof useEngine>; hw: HardwareView | null }) {
  const { status, error, busy, group } = engine;
  const backend = status?.installed ? status.backend : hw?.backend;
  return (
    <div className="rounded-xl border border-neutral-200 bg-white p-5 shadow-sm dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="font-medium">Image engine</div>
          <div className="text-sm text-neutral-600 dark:text-neutral-400">Version for {backendPlain(backend)}</div>
        </div>
        {status?.installed && !busy ? (
          <span className="inline-flex items-center gap-1.5 text-sm font-medium text-emerald-700 dark:text-emerald-400">
            <CircleCheck className="h-4 w-4" /> Ready
          </span>
        ) : !busy && status ? (
          <Button variant="primary" onClick={() => void engine.install()}>
            {error ? <RotateCw className="h-4 w-4" /> : <Download className="h-4 w-4" />}
            {error ? "Try again" : "Download engine"}
          </Button>
        ) : !status ? (
          <Spinner className="h-5 w-5 text-neutral-400" />
        ) : null}
      </div>
      {busy && (
        <div className="mt-4">
          {group ? (
            <GroupProgress group={group} onCancel={() => void cancelGroup(group.groupId)} />
          ) : (
            <div className="flex items-center gap-2 text-sm text-neutral-500">
              <Spinner className="h-4 w-4 text-amber-500" /> Starting download…
            </div>
          )}
        </div>
      )}
      {status?.installed && !busy && status.version && <p className="mt-2 text-xs text-neutral-500">Version {status.version}</p>}
      {error && (
        <div className="mt-4">
          <ErrorNotice error={error} />
        </div>
      )}
      {!status?.installed && !busy && (
        <p className="mt-4 text-xs text-neutral-500">
          The app needs the engine to make pictures. If you skip this, you can download it later in Settings → Engine.
        </p>
      )}
    </div>
  );
}
