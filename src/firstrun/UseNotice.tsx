// First-launch "Before you start" notice (RELEASE-SPEC §7): what not to make, and that a
// local check looks at every picture. Shown before anything else until the user agrees;
// only the notice version is stored (settings.noticeAccepted).
import { useState } from "react";
import { ScanEye, ShieldCheck } from "lucide-react";
import { asCoreError, getSettings, quitApp, setSettings } from "../lib/api";
import type { CoreError } from "../lib/types";
import { Button, ErrorNotice, Spinner } from "../components/ui";
import { Logo } from "../components/Logo";
import { emitSettingsChanged } from "../settings/events";

/** Bump when the notice changes in a way people must see again. */
export const NOTICE_VERSION = 1;

export function UseNotice(props: { onAgreed: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);

  const agree = async () => {
    setBusy(true);
    setError(null);
    try {
      const current = await getSettings();
      const saved = await setSettings({ ...current, noticeAccepted: NOTICE_VERSION });
      emitSettingsChanged(saved);
      props.onAgreed();
    } catch (e) {
      setError(asCoreError(e));
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-40 overflow-y-auto bg-neutral-50 text-neutral-900 dark:bg-neutral-950 dark:text-neutral-100">
      <div className="mx-auto flex min-h-full max-w-xl flex-col justify-center px-6 py-10">
        <div className="flex items-center gap-2 font-semibold">
          <Logo className="h-7 w-7" /> Pinhole
        </div>
        <h1 className="mt-8 text-2xl font-semibold tracking-tight">Before you start</h1>
        <p className="mt-3 flex items-start gap-3 text-sm text-neutral-600 dark:text-neutral-400">
          <ShieldCheck className="mt-0.5 h-5 w-5 shrink-0 text-emerald-600" />
          Pinhole makes pictures on your computer. Your prompts and images stay on your computer.
        </p>

        <h2 className="mt-6 text-sm font-semibold">Please don't use Pinhole to make:</h2>
        <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-neutral-700 dark:text-neutral-300">
          <li>sexual images of anyone under 18, or of anyone who looks under 18</li>
          <li>sexual or intimate images of a real person without their consent</li>
          <li>fake pictures of real people meant to deceive, embarrass or harass them</li>
          <li>fake documents, IDs, receipts or evidence</li>
        </ul>
        <p className="mt-3 text-sm text-neutral-600 dark:text-neutral-400">
          You're responsible for the pictures you make, for following the laws where you live, and for each model's
          licence.
        </p>

        <div className="mt-6 rounded-xl border border-neutral-200 bg-white p-4 text-sm dark:border-neutral-800 dark:bg-neutral-900">
          <h2 className="flex items-center gap-2 font-semibold">
            <ScanEye className="h-4 w-4" /> Pinhole checks the pictures it makes
          </h2>
          <p className="mt-2 text-neutral-600 dark:text-neutral-400">
            A small image-checking model runs on your computer and looks at each picture before it's shown. It stops
            sexual images of anyone who looks under 18, nude or intimate edits of photos of real people, and adult images
            from models marked for safe images only.
          </p>
          <p className="mt-2 text-neutral-600 dark:text-neutral-400">
            The check works offline and can't be turned off. Nothing about it is saved or sent anywhere. It can make
            mistakes; when it stops a picture, your prompt and settings are kept.
          </p>
        </div>

        {error && (
          <div className="mt-4">
            <ErrorNotice error={error} />
          </div>
        )}

        <div className="mt-8 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => void quitApp().catch(() => undefined)} disabled={busy}>
            Quit
          </Button>
          <Button variant="primary" size="lg" onClick={() => void agree()} disabled={busy}>
            {busy && <Spinner className="h-4 w-4" />} I agree
          </Button>
        </div>
      </div>
    </div>
  );
}
