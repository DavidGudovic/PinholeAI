// "What goes online": the complete list of what Pinhole sends over the internet, and when.
// Keep it in step with the code: every network call goes through pinhole_net::HttpClient
// (host allow-list + Offline mode), so this list is that client's list of callers.
import { useState } from "react";
import { ShieldCheck, Wifi, WifiOff } from "lucide-react";
import * as api from "../lib/api";
import { useAppState } from "../lib/state/store";
import { emitSettingsChanged } from "../settings/events";
import type { CoreError } from "../lib/types";
import { Button, Dialog, ErrorNotice, cx, focusRing } from "./ui";

interface Call {
  when: string;
  where: string;
  sent: string;
}

/** Everything that can go online, each only after you do something. */
export const ONLINE_CALLS: Call[] = [
  {
    when: "You open Models and browse or search",
    where: "civitai.com (pictures from image.civitai.com)",
    sent: "Your search words and filters. Your CivitAI key, if you added one, goes in the request header.",
  },
  {
    when: "You open a model’s page",
    where: "civitai.com",
    sent: "Which model you opened.",
  },
  {
    when: "You start a download: a model or add-on, the engine and its safety check, a Describe or Improve helper, or the upscaler the first time you use Upscale",
    where: "civitai.com, huggingface.co or github.com, and their download servers",
    sent: "A request for that file. Nothing about your pictures or prompts.",
  },
  {
    when: "You paste generation data from CivitAI, or use a model page’s settings",
    where: "civitai.com",
    sent: "Only the model and add-on numbers or file fingerprints, to find them. The prompt stays on this computer.",
  },
  {
    when: "You add a model file you already have that Pinhole doesn’t recognise",
    where: "civitai.com",
    sent: "The file’s fingerprint (a SHA-256 hash), to find its name. Not the file.",
  },
  {
    when: "You press Check for updates in Settings",
    where: "api.github.com and github.com",
    sent: "A request for the release list. Your GitHub token, if you added one, goes in the request header.",
  },
];

/** Stays on this computer, always. */
export const STAYS_LOCAL = [
  "Your prompts, pictures, styles and settings.",
  "Making pictures, editing and Describe: the engines run on this computer and listen on this computer only.",
  "No usage data, crash reports, analytics or automatic update checks. Nothing goes online until you do something above.",
  "The app window itself makes no network calls of its own: no web fonts, no remote images.",
];

function Body({ offline }: { offline: boolean }) {
  return (
    <div className="space-y-4 text-sm">
      <p className={cx("flex items-start gap-2 rounded-lg px-3 py-2", offline ? "bg-emerald-50 text-emerald-900 dark:bg-emerald-500/10 dark:text-emerald-200" : "bg-neutral-100 text-neutral-700 dark:bg-neutral-800/60 dark:text-neutral-300")}>
        {offline ? <WifiOff className="mt-0.5 h-4 w-4 shrink-0" /> : <Wifi className="mt-0.5 h-4 w-4 shrink-0" />}
        <span>
          {offline
            ? "Offline mode is on. Pinhole blocks every internet request below until you turn it off."
            : "Offline mode is off. Pinhole goes online only for the things listed here, and only when you ask for them."}
        </span>
      </p>

      <section aria-label="Goes online only when you ask">
        <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-neutral-500 uppercase">Goes online only when you…</h3>
        <ul className="divide-y divide-neutral-200 rounded-lg border border-neutral-200 dark:divide-neutral-800 dark:border-neutral-800">
          {ONLINE_CALLS.map((c) => (
            <li key={c.when} className="space-y-0.5 px-3 py-2">
              <div className="font-medium">{c.when}</div>
              <div className="text-xs text-neutral-500">
                <span className="font-medium text-neutral-600 dark:text-neutral-400">Talks to:</span> {c.where}
              </div>
              <div className="text-xs text-neutral-500">
                <span className="font-medium text-neutral-600 dark:text-neutral-400">Sends:</span> {c.sent}
              </div>
            </li>
          ))}
        </ul>
        <p className="mt-1.5 text-xs text-neutral-500">Pinhole can reach only these sites: civitai.com, huggingface.co and github.com, plus the servers they hand their downloads to.</p>
      </section>

      <section aria-label="Stays on this computer">
        <h3 className="mb-1.5 text-xs font-semibold tracking-wide text-neutral-500 uppercase">Stays on this computer</h3>
        <ul className="space-y-1.5">
          {STAYS_LOCAL.map((t) => (
            <li key={t} className="flex items-start gap-2">
              <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
              <span>{t}</span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

/** The sheet itself. `onToggle` adds a button that switches Offline mode (left out in Settings, which has the switch). */
export function WhatGoesOnline({ open, onClose, offline, onToggle }: { open: boolean; onClose: () => void; offline: boolean; onToggle?: () => Promise<void> }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<CoreError | null>(null);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="What goes online"
      footer={
        <>
          {onToggle && (
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => {
                setBusy(true);
                setError(null);
                onToggle()
                  .catch((e) => setError(api.asCoreError(e)))
                  .finally(() => setBusy(false));
              }}
            >
              {offline ? "Turn Offline mode off" : "Turn Offline mode on"}
            </Button>
          )}
          <Button variant="primary" onClick={onClose}>
            Close
          </Button>
        </>
      }
    >
      <Body offline={offline} />
      {error && <ErrorNotice error={error} onDismiss={() => setError(null)} />}
    </Dialog>
  );
}

/** Top-bar badge: "Offline" or "Online", opening the sheet. */
export function OnlineBadge() {
  // Nothing until the settings are loaded: "Online" would be a guess.
  const loaded = useAppState((s) => s.settings?.offline);
  const offline = loaded ?? false;
  const [open, setOpen] = useState(false);
  const toggle = async () => emitSettingsChanged(await api.setSettings({ ...(await api.getSettings()), offline: !offline }));
  if (loaded === undefined) return null;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title={offline ? "Offline mode is on. Click to see what goes online." : "Online only when you ask. Click to see what goes online."}
        className={cx(
          "inline-flex h-8 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium",
          focusRing,
          offline
            ? "bg-emerald-50 text-emerald-800 hover:bg-emerald-100 dark:bg-emerald-500/10 dark:text-emerald-300 dark:hover:bg-emerald-500/20"
            : "text-neutral-500 hover:bg-neutral-100 hover:text-neutral-900 dark:text-neutral-400 dark:hover:bg-neutral-800 dark:hover:text-white",
        )}
      >
        {offline ? <WifiOff className="h-3.5 w-3.5" /> : <Wifi className="h-3.5 w-3.5" />}
        {offline ? "Offline" : "Online"}
        <span className="sr-only"> (what goes online)</span>
      </button>
      <WhatGoesOnline open={open} onClose={() => setOpen(false)} offline={offline} onToggle={toggle} />
    </>
  );
}

/** Link for Settings → Privacy (the Offline switch is right there, so the sheet has no switch). */
export function WhatGoesOnlineLink({ offline }: { offline: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} className={cx("rounded text-xs font-medium text-amber-700 underline underline-offset-2 hover:text-amber-900 dark:text-amber-400 dark:hover:text-amber-200", focusRing)}>
        What goes online
      </button>
      <WhatGoesOnline open={open} onClose={() => setOpen(false)} offline={offline} />
    </>
  );
}
