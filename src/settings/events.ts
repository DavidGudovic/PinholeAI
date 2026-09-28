// In-page notification when Settings are saved from this WebView (Settings sheet,
// First run, "Add trigger words" toggle). The shell can listen to apply the theme
// immediately; the Models tab uses it to react to Offline mode. Settings hold no
// prompt text, so passing them around in memory is fine.
import type { Settings } from "../lib/types";

export const SETTINGS_CHANGED_EVENT = "pinhole:settings-changed";

export function emitSettingsChanged(settings: Settings) {
  window.dispatchEvent(new CustomEvent<Settings>(SETTINGS_CHANGED_EVENT, { detail: settings }));
}

/** Returns an unsubscribe function. */
export function onSettingsChanged(cb: (settings: Settings) => void): () => void {
  const handler = (e: Event) => cb((e as CustomEvent<Settings>).detail);
  window.addEventListener(SETTINGS_CHANGED_EVENT, handler);
  return () => window.removeEventListener(SETTINGS_CHANGED_EVENT, handler);
}
