// First-launch "Before you start" notice (RELEASE-SPEC §7): agree to the usage guidelines
// (full rules in <UsageGuidelines />) and know that a local check looks at every picture.
// Shown before anything else until the user agrees; only the notice version is stored
// (settings.noticeAccepted).
import { useState } from "react";
import { ShieldCheck } from "lucide-react";
import { getSettings, quitApp, setSettings } from "../lib/api";
import { Button, cx, focusRing, Spinner } from "../components/ui";
import { UsageGuidelines } from "../components/UsageGuidelines";
import { Logo } from "../components/Logo";
import { emitSettingsChanged } from "../settings/events";

/** Bump when the notice changes in a way people must see again. */
export const NOTICE_VERSION = 1;

export function UseNotice(props: { onAgreed: () => void }) {
  const [busy, setBusy] = useState(false);
  const [guidelinesOpen, setGuidelinesOpen] = useState(false);

  const agree = async () => {
    setBusy(true);
    try {
      const current = await getSettings();
      const saved = await setSettings({ ...current, noticeAccepted: NOTICE_VERSION });
      emitSettingsChanged(saved);
      props.onAgreed();
    } catch {
      // Settings can't be read or saved (the app already runs on fallback settings then):
      // continue for this session; the notice shows again next launch.
      props.onAgreed();
    }
  };

  return (
    <div className="fixed inset-0 z-40 overflow-y-auto bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <div className="mx-auto flex min-h-full max-w-md flex-col justify-center px-6 py-10">
        <Logo className="h-10 w-10" />
        <h1 className="mt-6 text-2xl font-semibold tracking-tight">Before you start</h1>
        <p className="mt-3 text-sm text-neutral-600 dark:text-neutral-400">
          Pinhole makes pictures on your computer. Your prompts and images stay on your computer.
        </p>

        <div className="mt-6 rounded-xl border border-neutral-200 bg-white p-4 text-sm dark:border-neutral-800 dark:bg-neutral-900">
          <h2 className="flex items-center gap-2 font-semibold">
            <ShieldCheck className="h-4 w-4 text-emerald-600" /> Built-in safety check
          </h2>
          <p className="mt-2 text-neutral-600 dark:text-neutral-400">
            Pinhole checks prompts and pictures on your computer and stops the most harmful content, such as sexual
            images of anyone who looks under 18. The check works offline, can't be turned off and keeps no record.
          </p>
        </div>

        <p className="mt-6 text-sm text-neutral-600 dark:text-neutral-400">
          By continuing, you agree to the{" "}
          <button
            type="button"
            className={cx("rounded font-medium text-neutral-900 underline underline-offset-2 dark:text-neutral-100", focusRing)}
            onClick={() => setGuidelinesOpen(true)}
          >
            usage guidelines
          </button>{" "}
          and to each model's licence. You're responsible for what you make.
        </p>

        <div className="mt-8 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => void quitApp().catch(() => undefined)} disabled={busy}>
            Quit
          </Button>
          <Button variant="primary" size="lg" onClick={() => void agree()} disabled={busy}>
            {busy && <Spinner className="h-4 w-4" />} Agree and continue
          </Button>
        </div>
      </div>
      <UsageGuidelines open={guidelinesOpen} onClose={() => setGuidelinesOpen(false)} />
    </div>
  );
}
